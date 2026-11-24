#[cfg(test)]
fn key_id(key: &SigKey) -> u32 {
    match key {
        SigKey::Can { id, .. } => *id,
        SigKey::Fr { slot, .. } => *slot as u32,
        SigKey::Eth { .. } | SigKey::Derived { .. } => 0,
    }
}

fn bus_name(app: &App, ch: u8) -> String {
    app.snap
        .channels
        .get(ch as usize)
        .map(|channel| channel.name.clone())
        .unwrap_or_else(|| format!("CAN{}", ch + 1))
}

fn signal_unit(app: &App, ch: u8, id: u32, ext: bool, name: &str) -> String {
    app.snap
        .channels
        .get(ch as usize)
        .and_then(|channel| channel.dbc.as_ref())
        .and_then(|db| db.messages.get(&(id, ext)))
        .and_then(|message| message.signals.iter().find(|signal| signal.name == name))
        .map(|signal| signal.unit.clone())
        .unwrap_or_default()
}

fn signal_sub<'a>(app: &'a App, key: &SigKey) -> Option<&'a SubView> {
    app.snap.subs.iter().find(|sub| &sub.key == key)
}

/// Resolves the address every observer command carries. CAN keys use a
/// message id; FlexRay keys use a slot in the same `id` field with the
/// cluster in `ch`; Ethernet names its frame. One command shape covers the
/// three bus kinds.
fn command_key(
    app: &crate::bus::BusCore,
    kind: &str,
    ch: u8,
    id: u32,
    ext: bool,
    frame: Option<String>,
    name: String,
) -> Option<SigKey> {
    match kind {
        "can" => Some(app.can_key(ch, id, ext, name)),
        "fr" => u16::try_from(id)
            .ok()
            .map(|slot| app.fr_key(ch, slot, name)),
        "eth" => frame.map(|frame| app.eth_key(ch, frame, name)),
        "derived" => frame.map(|id| SigKey::derived(id, name)),
        _ => None,
    }
}

struct RowAddress {
    kind: &'static str,
    ch: u8,
    id: u32,
    ext: bool,
    frame: Option<String>,
}

/// Observer rows carry one address shape for every bus kind. FlexRay puts
/// its cluster in `ch` and its slot in `id`; Ethernet keeps `ch` and names
/// its frame.
fn row_address(app: &crate::bus::BusCore, key: &SigKey) -> RowAddress {
    match key {
        SigKey::Can { id, ext, .. } => RowAddress {
            kind: "can",
            ch: app.key_bus(key).unwrap_or(0),
            id: *id,
            ext: *ext,
            frame: None,
        },
        SigKey::Fr { slot, .. } => RowAddress {
            kind: "fr",
            ch: app.key_bus(key).unwrap_or(0),
            id: u32::from(*slot),
            ext: false,
            frame: None,
        },
        SigKey::Eth { frame, .. } => RowAddress {
            kind: "eth",
            ch: app.key_bus(key).unwrap_or(0),
            id: 0,
            ext: false,
            frame: Some(frame.clone()),
        },
        SigKey::Derived { id, .. } => RowAddress {
            kind: "derived",
            ch: 0,
            id: 0,
            ext: false,
            frame: Some(id.clone()),
        },
    }
}

pub(crate) fn series_key(key: &SigKey) -> String {
    key.text()
}

fn signal_bus(app: &App, key: &SigKey) -> String {
    match key {
        SigKey::Can { .. } => app
            .key_bus(key)
            .map(|ch| bus_name(app, ch))
            .unwrap_or_default(),
        SigKey::Fr { .. } => app
            .key_bus(key)
            .map(|bus| app.fr_bus_name(bus))
            .unwrap_or_default(),
        SigKey::Eth { .. } => app
            .key_bus(key)
            .map(|bus| app.eth_bus_label(bus))
            .unwrap_or_default(),
        SigKey::Derived { .. } => "Derived".to_string(),
    }
}

/// `Frame / PDU / Signal`, skipping a PDU whose name repeats the frame (DBC
/// imports name them the same). Rows can then tell apart same-named ARXML
/// signals that live in different frames or PDUs.
fn signal_path(app: &App, key: &SigKey) -> Option<String> {
    let bus = app.key_bus(key)?;
    match key {
        SigKey::Can {
            cluster,
            id,
            ext,
            name,
        } => {
            let comm = app.model_comms.get(bus as usize)?.as_ref()?;
            let frame = comm
                .frame_by_can_id(*cluster, *id)
                .into_iter()
                .find(|frame| {
                    matches!(
                        frame.address,
                        crate::model::FrameAddress::Can {
                            extended,
                            ..
                        } if extended == *ext
                    )
                })?;
            let signal = comm.signal_by_name(frame, name).into_iter().next()?;
            Some(model_signal_path(comm, frame, signal))
        }
        SigKey::Fr {
            cluster,
            slot,
            name,
        } => {
            let comm = &app.fr_models.get(&bus)?.comm;
            for frame in comm.frame_by_slot(*cluster, *slot) {
                if let Some(signal) = comm.signal_by_name(frame, name).into_iter().next() {
                    return Some(model_signal_path(comm, frame, signal));
                }
            }
            None
        }
        SigKey::Eth {
            cluster,
            frame,
            name,
        } => {
            let comm = &app.eth_models.get(&bus)?.comm;
            let frame = comm.frame_by_name(*cluster, frame)?;
            let signal = comm.signal_by_name(frame, name).into_iter().next()?;
            Some(model_signal_path(comm, frame, signal))
        }
        SigKey::Derived { .. } => None,
    }
}

fn model_signal_path(
    comm: &crate::model::Comm,
    frame: &crate::model::Frame,
    signal: &crate::model::Signal,
) -> String {
    let mut parts = Vec::with_capacity(3);
    if !frame.name.is_empty() {
        parts.push(frame.name.clone());
    }
    if let Some(pdu) = comm.pdus.get(&signal.pdu)
        && !pdu.name.is_empty()
        && pdu.name != frame.name
    {
        parts.push(pdu.name.clone());
    }
    parts.push(signal.name.clone());
    parts.join(" / ")
}

fn signal_meta(signal: &SelectedSignal) -> SignalSeries {
    SignalSeries {
        key: series_key(&signal.key),
        name: signal.name.clone(),
        bus: signal.bus.clone(),
        unit: signal.unit.clone(),
        color: signal.color,
        label: None,
        read: signal.read,
        points: Vec::new(),
    }
}

