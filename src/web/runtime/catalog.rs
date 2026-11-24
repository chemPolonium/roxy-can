/// Build the signal tree for the picker: every CAN channel with a DBC plus
/// every loaded FlexRay cluster. Keys are the same strings the series
/// endpoint reports, so selection state round-trips without translation.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalog {
    channels: Vec<WebCatalogChannel>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalogChannel {
    kind: String,
    index: u8,
    name: String,
    /// Cluster names the description carries, so the editor can offer the
    /// named FlexRay/Ethernet spellings (`FR.PowerTrain...`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    aliases: Vec<String>,
    messages: Vec<WebCatalogMessage>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalogMessage {
    id: u32,
    extended: bool,
    name: String,
    /// Ethernet frames are addressed by name, not by their catalog index.
    #[serde(skip_serializing_if = "Option::is_none")]
    frame: Option<String>,
    /// `CM_ BO_` description; omitted when the database has none.
    #[serde(skip_serializing_if = "String::is_empty")]
    comment: String,
    /// CAN FD, either declared by `VFrameFormat` or implied by a long DLC.
    fd: bool,
    /// Ethernet payload bytes after the project's L2-header convention.
    #[serde(skip_serializing_if = "Option::is_none")]
    length: Option<u32>,
    signals: Vec<WebCatalogSignal>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCatalogSignal {
    key: String,
    name: String,
    unit: String,
    min: f64,
    max: f64,
    /// Declared start value in physical units; the physical send mode shows
    /// it as the placeholder for an untouched field.
    #[serde(skip_serializing_if = "Option::is_none")]
    initial: Option<f64>,
    /// `CM_ SG_` description; omitted when the database has none.
    #[serde(skip_serializing_if = "String::is_empty")]
    comment: String,
    /// The I-SIGNAL-GROUP this signal belongs to, when one names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    group: Option<String>,
}

fn channel_signature(app: &App) -> String {
    let parts: Vec<String> = app
        .snap
        .channels
        .iter()
        .map(|channel| {
            let table = channel
                .dbc
                .as_ref()
                .map(|db| Arc::as_ptr(db) as usize)
                .unwrap_or(0);
            format!("{}:{}:{table}", channel.name, channel.dbc_paths.join(","))
        })
        .enumerate()
        .map(|(index, text)| {
            let model = app
                .model_comms
                .get(index)
                .and_then(Option::as_ref)
                .map(|comm| Arc::as_ptr(comm) as usize)
                .unwrap_or(0);
            format!("{text}:{model}")
        })
        .collect();
    let fr: Vec<String> = app
        .fr_buses
        .iter()
        .map(|(bus, cfg)| {
            format!(
                "fr{bus}:{}:{}",
                app.fr_description_path(*bus).unwrap_or_default(),
                Arc::as_ptr(cfg) as usize
            )
        })
        .collect();
    let eth: Vec<String> = app
        .eth_buses
        .iter()
        .map(|(bus, cfg)| {
            format!(
                "eth{bus}:{}:{}",
                app.eth_description_path(*bus).unwrap_or_default(),
                Arc::as_ptr(cfg) as usize
            )
        })
        .collect();
    let someip: Vec<String> = app
        .someip_services
        .services
        .iter()
        .map(|service| {
            format!(
                "someip{}:{}:{}:{}",
                service.id,
                service.methods.len(),
                service.events.len(),
                service.instances.len()
            )
        })
        .collect();
    parts
        .into_iter()
        .chain(fr)
        .chain(eth)
        .chain(someip)
        .collect::<Vec<_>>()
        .join(";")
}

fn refresh_catalog(app: &App, catalog: &Arc<Mutex<WebCatalog>>, signature: &mut String) {
    let next = channel_signature(app);
    if next == *signature {
        return;
    }
    *signature = next;
    if let Ok(mut slot) = catalog.lock() {
        *slot = build_catalog(app);
    }
}

fn catalog_signal(
    key: String,
    signal: &crate::model::Signal,
    group: Option<String>,
) -> WebCatalogSignal {
    WebCatalogSignal {
        key,
        name: signal.name.clone(),
        unit: signal.unit.clone(),
        min: signal.min.unwrap_or(0.0),
        max: signal.max.unwrap_or(0.0),
        initial: signal.initial_physical(),
        comment: signal.comment.clone(),
        group,
    }
}

/// The name of the I-SIGNAL-GROUP a signal belongs to, from the model.
fn signal_group_name(comm: &crate::model::Comm, signal: &crate::model::Signal) -> Option<String> {
    signal
        .group
        .and_then(|group| comm.groups.get(&group))
        .map(|group| group.name.clone())
}

/// The messages one CAN channel's model declares, in id order. Names,
/// comments, signals and ranges all come from the model.
fn can_catalog_messages(comm: &crate::model::Comm) -> Vec<WebCatalogMessage> {
    let cluster = comm
        .clusters
        .keys()
        .next()
        .copied()
        .unwrap_or(crate::model::ClusterId(0));
    let mut out: Vec<WebCatalogMessage> = comm
        .frames
        .values()
        .filter_map(|frame| {
            let crate::model::FrameAddress::Can { id, extended, fd } = frame.address else {
                return None;
            };
            Some(WebCatalogMessage {
                id,
                extended,
                name: frame.name.clone(),
                frame: None,
                comment: frame.comment.clone(),
                fd,
                length: None,
                signals: comm
                    .signals_in_frame(frame)
                    .into_iter()
                    .map(|signal| {
                        catalog_signal(
                            series_key(&SigKey::can_in(cluster, id, extended, &signal.name)),
                            signal,
                            signal_group_name(comm, signal),
                        )
                    })
                    .collect(),
            })
        })
        .collect();
    out.sort_by_key(|message| (message.id, message.extended));
    out
}

fn fr_catalog_messages(comm: &crate::model::Comm) -> Vec<WebCatalogMessage> {
    let cluster = comm
        .clusters
        .keys()
        .next()
        .copied()
        .unwrap_or(crate::model::ClusterId(0));
    let mut out: Vec<WebCatalogMessage> = comm
        .frames
        .values()
        .filter_map(|frame| {
            let crate::model::FrameAddress::FlexRay { slot, .. } = frame.address else {
                return None;
            };
            Some(WebCatalogMessage {
                id: u32::from(slot),
                extended: false,
                name: frame.name.clone(),
                frame: None,
                comment: String::new(),
                fd: false,
                length: None,
                signals: comm
                    .signals_in_frame(frame)
                    .into_iter()
                    .map(|signal| {
                        catalog_signal(
                            series_key(&SigKey::fr_in(cluster, slot, &signal.name)),
                            signal,
                            signal_group_name(comm, signal),
                        )
                    })
                    .collect(),
            })
        })
        .collect();
    out.sort_by_key(|message| message.id);
    out
}

fn eth_catalog_messages(
    comm: &crate::model::Comm,
    length_includes_header: bool,
) -> Vec<WebCatalogMessage> {
    let cluster = comm
        .clusters
        .keys()
        .next()
        .copied()
        .unwrap_or(crate::model::ClusterId(0));
    comm.frames
        .values()
        .filter(|frame| matches!(frame.address, crate::model::FrameAddress::Ethernet { .. }))
        .enumerate()
        .map(|(index, frame)| WebCatalogMessage {
            id: index as u32,
            extended: false,
            name: frame.name.clone(),
            frame: Some(frame.name.clone()),
            comment: String::new(),
            fd: false,
            length: Some(if length_includes_header {
                frame.length.saturating_sub(14)
            } else {
                frame.length
            }),
            signals: comm
                .signals_in_frame(frame)
                .into_iter()
                .map(|signal| {
                    catalog_signal(
                        series_key(&SigKey::eth_in(cluster, &frame.name, &signal.name)),
                        signal,
                        signal_group_name(comm, signal),
                    )
                })
                .collect(),
        })
        .collect()
}

fn catalog_aliases(comm: &crate::model::Comm) -> Vec<String> {
    let mut names: Vec<String> = comm
        .clusters
        .values()
        .map(|cluster| cluster.name.clone())
        .filter(|name| !name.is_empty())
        .collect();
    names.dedup();
    names
}

fn someip_signal_names(prefix: &str, ty: &crate::eth::arxml_service::ServiceType) -> Vec<String> {
    use crate::eth::arxml_service::ServiceType;
    match ty {
        ServiceType::Record(fields) => fields
            .iter()
            .flat_map(|field| someip_signal_names(&format!("{prefix}.{}", field.name), &field.ty))
            .collect(),
        ServiceType::Array { element, count } => (0..*count)
            .flat_map(|index| someip_signal_names(&format!("{prefix}[{index}]"), element))
            .collect(),
        _ => vec![prefix.to_string()],
    }
}

/// The editable leaves of one SOME/IP field, with the type facts the Panel
/// needs to render the right input. It mirrors `someip_signal_names`, but
/// keeps enum labels and scalar/text kinds.
fn someip_field_defs(
    prefix: &str,
    field: &crate::eth::arxml_service::ServiceField,
) -> Vec<WebSomeIpField> {
    let mut out = Vec::new();
    someip_collect_field_defs(prefix, &field.ty, &field.props, &mut out);
    out
}

fn someip_collect_field_defs(
    prefix: &str,
    ty: &crate::eth::arxml_service::ServiceType,
    props: &crate::eth::arxml_service::SomeIpProps,
    out: &mut Vec<WebSomeIpField>,
) {
    use crate::eth::arxml_service::ServiceType;
    let numeric = |name: &str, props: &crate::eth::arxml_service::SomeIpProps| WebSomeIpField {
        name: name.to_string(),
        kind: "number".to_string(),
        unit: props.unit.clone(),
        min: props.min.as_deref().and_then(|value| value.parse().ok()),
        max: props.max.as_deref().and_then(|value| value.parse().ok()),
        labels: Vec::new(),
    };
    match ty {
        ServiceType::Record(fields) => {
            for field in fields {
                someip_collect_field_defs(
                    &format!("{prefix}.{}", field.name),
                    &field.ty,
                    &field.props,
                    out,
                );
            }
        }
        ServiceType::Array { element, count } => {
            for index in 0..*count {
                someip_collect_field_defs(&format!("{prefix}[{index}]"), element, props, out);
            }
        }
        ServiceType::Enum { labels, .. } => {
            let mut field = numeric(prefix, props);
            field.labels = labels.clone();
            out.push(field);
        }
        ServiceType::String => out.push(WebSomeIpField {
            name: prefix.to_string(),
            kind: "text".to_string(),
            unit: props.unit.clone(),
            min: None,
            max: None,
            labels: Vec::new(),
        }),
        ServiceType::Union(variants) => {
            out.push(numeric(&format!("{prefix}.$selector"), props));
            for variant in variants {
                someip_collect_field_defs(
                    &format!("{prefix}.{}", variant.name),
                    &variant.ty,
                    &variant.props,
                    out,
                );
            }
        }
        _ => out.push(numeric(prefix, props)),
    }
}

fn someip_catalog_messages(app: &App, bus: u8) -> Vec<WebCatalogMessage> {
    let mut out = Vec::new();
    for service in &app.someip_services.services {
        if !service.buses.is_empty() && !service.buses.contains(&bus) {
            continue;
        }
        let mut add =
            |name: &str, method: u16, fields: &[crate::eth::arxml_service::ServiceField]| {
                if fields.is_empty() {
                    return;
                }
                let frame = format!("SOMEIP 0x{:04X}/0x{:04X}", service.id, method);
                let signals = fields
                    .iter()
                    .filter(|field| !field.name.is_empty())
                    .flat_map(|field| {
                        someip_signal_names(&field.name, &field.ty)
                            .into_iter()
                            .map(|signal| WebCatalogSignal {
                                key: series_key(&app.eth_key(bus, frame.clone(), signal.clone())),
                                name: signal,
                                unit: field.props.unit.clone(),
                                min: field
                                    .props
                                    .min
                                    .as_deref()
                                    .and_then(|value| value.parse().ok())
                                    .unwrap_or(0.0),
                                max: field
                                    .props
                                    .max
                                    .as_deref()
                                    .and_then(|value| value.parse().ok())
                                    .unwrap_or(0.0),
                                initial: None,
                                comment: String::new(),
                                group: None,
                            })
                    })
                    .collect();
                out.push(WebCatalogMessage {
                    id: (u32::from(service.id) << 16) | u32::from(method),
                    extended: true,
                    name: name.to_string(),
                    frame: Some(frame),
                    comment: format!("SOME/IP service 0x{:04X}", service.id),
                    fd: false,
                    length: None,
                    signals,
                });
            };
        for event in &service.events {
            add(&event.name, event.id, &event.fields);
        }
        for method in &service.methods {
            let mut fields = method.input.clone();
            fields.extend(method.output.iter().cloned());
            add(&method.name, method.id, &fields);
        }
    }
    out
}

