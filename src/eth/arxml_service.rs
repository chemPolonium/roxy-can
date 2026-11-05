//! SOME/IP service catalog read from AUTOSAR ARXML.
#![allow(dead_code)] // catalog/codec land before the SOME/IP payload view uses them
//!
//! The service interface itself carries methods/events/fields and their
//! argument types; the deployment object carries the numeric service id and
//! the method/event ids. Both live under package-level ELEMENTS, so this
//! scanner indexes every element by its full path first and resolves the
//! deployment references afterwards.
//!
//! Scope: primitive value types, booleans and records. Arrays, unions and
//! transformer alignment are rejected with an issue rather than decoded
//! wrongly. Payloads use SOME/IP's network byte order and are packed by the
//! type sizes; a real transformer may insert padding, which is why this
//! catalog is only used when the whole layout resolved.

use roxmltree::{Document, Node};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceType {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Bool,
    Record(Vec<ServiceField>),
    /// Fixed-size array of one element type. Dynamic arrays are rejected
    /// until their length field/strategy is implemented.
    Array {
        element: Box<ServiceType>,
        count: usize,
    },
    /// Numeric base plus its TEXTTABLE labels (AUTOSAR enum).
    Enum {
        base: Box<ServiceType>,
        labels: Vec<(i64, String)>,
    },
    /// SOME/IP default string: 32-bit length, UTF-8 bytes, optional NUL.
    String,
    /// SOME/IP default union: 32-bit length, 32-bit type selector, payload.
    Union(Vec<ServiceField>),
}

impl ServiceType {
    /// Fixed wire size in bytes, when known. Dynamic strings/unions and
    /// dynamic arrays have no fixed size.
    fn fixed_size(&self) -> Option<usize> {
        match self {
            ServiceType::U8 | ServiceType::I8 | ServiceType::Bool => Some(1),
            ServiceType::U16 | ServiceType::I16 => Some(2),
            ServiceType::U32 | ServiceType::I32 | ServiceType::F32 => Some(4),
            ServiceType::U64 | ServiceType::I64 | ServiceType::F64 => Some(8),
            ServiceType::Record(fields) => fields
                .iter()
                .try_fold(0usize, |sum, field| sum.checked_add(field.ty.fixed_size()?)),
            ServiceType::Array { element, count } => element
                .fixed_size()
                .and_then(|size| size.checked_mul(*count)),
            ServiceType::Enum { base, .. } => base.fixed_size(),
            ServiceType::String | ServiceType::Union(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceField {
    pub name: String,
    pub ty: ServiceType,
    pub props: SomeIpProps,
}

/// SOME/IP transformer settings attached to one data prototype. Defaults are
/// the AUTOSAR/SOME/IP common 32-bit length fields and packed layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SomeIpProps {
    pub string_length_field: usize,
    pub union_length_field: usize,
    pub union_type_selector_field: usize,
    pub array_length_field: Option<usize>,
    pub struct_length_field: Option<usize>,
    pub alignment_bits: usize,
    pub little_endian: bool,
    pub unit: String,
    pub min: Option<String>,
    pub max: Option<String>,
}

impl Default for SomeIpProps {
    fn default() -> Self {
        Self {
            string_length_field: 4,
            union_length_field: 4,
            union_type_selector_field: 4,
            array_length_field: None,
            struct_length_field: None,
            alignment_bits: 0,
            little_endian: false,
            unit: String::new(),
            min: None,
            max: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceMethod {
    pub name: String,
    pub id: u16,
    pub input: Vec<ServiceField>,
    pub output: Vec<ServiceField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceEvent {
    pub name: String,
    pub id: u16,
    pub fields: Vec<ServiceField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceInstance {
    pub instance: u16,
    pub major: u8,
    pub minor: u32,
    pub eventgroups: Vec<u16>,
    /// `ip:port` of the owning application endpoint, when the deployment
    /// names one. Empty for interface-only deployments.
    pub endpoint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceDef {
    pub name: String,
    pub id: u16,
    pub major: u8,
    pub methods: Vec<ServiceMethod>,
    pub events: Vec<ServiceEvent>,
    pub instances: Vec<ServiceInstance>,
    /// Ethernet buses this service was imported on; empty when the catalog
    /// came from a standalone scan.
    pub buses: Vec<u8>,
    /// Unsupported pieces, surfaced by the import UI rather than swallowed.
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceCatalog {
    pub services: Vec<ServiceDef>,
}

impl ServiceCatalog {
    pub fn service(&self, id: u16) -> Option<&ServiceDef> {
        self.services.iter().find(|service| service.id == id)
    }

    pub fn method(&self, service: u16, method: u16) -> Option<&ServiceMethod> {
        self.service(service)?
            .methods
            .iter()
            .find(|entry| entry.id == method)
    }
}

/// Scans every `SERVICE-INTERFACE` and joins it with a matching
/// `SOMEIP-SERVICE-INTERFACE-DEPLOYMENT`.
pub fn scan(text: &str) -> Result<ServiceCatalog, String> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let doc = Document::parse(text).map_err(|error| format!("XML parse error: {error}"))?;
    let mut paths: Vec<(String, Node)> = Vec::new();
    for node in doc.descendants().filter(Node::is_element) {
        if let Some(path) = element_path(node) {
            paths.push((path, node));
        }
    }
    let props = transformation_props(&paths);
    let prototype_props = prototype_transformation_props(&paths, &props);
    let mut catalog = ServiceCatalog::default();
    for (path, node) in &paths {
        if node.tag_name().name() != "SERVICE-INTERFACE" {
            continue;
        }
        let name = short_name(*node).to_string();
        let Some(deployment) = paths.iter().find(|(_, candidate)| {
            candidate.tag_name().name() == "SOMEIP-SERVICE-INTERFACE-DEPLOYMENT"
                && child_text(*candidate, "SERVICE-INTERFACE-REF")
                    .is_some_and(|reference| path == &reference || path.ends_with(&reference))
        }) else {
            continue;
        };
        let Some(id) = child_text(deployment.1, "SERVICE-INTERFACE-ID")
            .and_then(parse_num)
            .and_then(|value| u16::try_from(value).ok())
        else {
            continue;
        };
        let mut service = ServiceDef {
            name,
            id,
            major: 0,
            methods: Vec::new(),
            events: Vec::new(),
            instances: Vec::new(),
            buses: Vec::new(),
            issues: Vec::new(),
        };
        collect_methods(&doc, *node, &paths, &prototype_props, &mut service);
        collect_events(&doc, *node, &paths, &prototype_props, &mut service);
        collect_fields(&doc, *node, &paths, &prototype_props, &mut service);
        collect_instances(&paths, &mut service);
        if !service.methods.is_empty() || !service.events.is_empty() {
            catalog.services.push(service);
        }
    }
    // Some Vector/ARXML descriptions carry only provided service instances
    // (service id, instance id, endpoint) without a SERVICE-INTERFACE. Keep
    // those as database objects so the Explorer is not empty.
    for (_, node) in &paths {
        if node.tag_name().name() != "PROVIDED-SERVICE-INSTANCE" {
            continue;
        }
        let Some(id) = child_text(*node, "SERVICE-IDENTIFIER")
            .and_then(parse_num)
            .and_then(|value| u16::try_from(value).ok())
        else {
            continue;
        };
        let instance = child_text(*node, "INSTANCE-IDENTIFIER")
            .and_then(parse_num)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(1);
        let major = node
            .descendants()
            .find(|child| child.has_tag_name("SERVER-SERVICE-MAJOR-VERSION"))
            .and_then(|child| child.text().map(str::to_string))
            .and_then(parse_num)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(1);
        let minor = node
            .descendants()
            .find(|child| child.has_tag_name("SERVER-SERVICE-MINOR-VERSION"))
            .and_then(|child| child.text().map(str::to_string))
            .and_then(parse_num)
            .unwrap_or(0);
        let name = short_name(*node).to_string();
        if let Some(service) = catalog.services.iter_mut().find(|service| service.id == id) {
            if !service
                .instances
                .iter()
                .any(|known| known.instance == instance)
            {
                service.instances.push(ServiceInstance {
                    instance,
                    major,
                    minor,
                    eventgroups: eventgroups_of(*node, &paths),
                    endpoint: endpoint_of(*node, &paths),
                });
            }
            continue;
        }
        let mut service = ServiceDef {
            name,
            id,
            major,
            methods: Vec::new(),
            events: Vec::new(),
            instances: vec![ServiceInstance {
                instance,
                major,
                minor,
                eventgroups: eventgroups_of(*node, &paths),
                endpoint: endpoint_of(*node, &paths),
            }],
            buses: Vec::new(),
            issues: vec![format!(
                "provided service instance 0x{id:04X} has no SERVICE-INTERFACE"
            )],
        };
        service.issues.shrink_to_fit();
        catalog.services.push(service);
    }
    catalog.services.sort_by_key(|service| service.id);
    Ok(catalog)
}

fn collect_methods(
    _doc: &Document,
    node: Node,
    paths: &[(String, Node)],
    prototype_props: &HashMap<String, SomeIpProps>,
    service: &mut ServiceDef,
) {
    for method in direct_children(node, "METHODS", "METHOD") {
        let name = short_name(method).to_string();
        let Some(id) = deployment_id(method, paths, "METHODS", "METHOD") else {
            service
                .issues
                .push(format!("method {} has no SOMEIP method id", name));
            continue;
        };
        let mut issues = Vec::new();
        let input = arguments(method, "INPUT", paths, prototype_props, &mut issues);
        let output = arguments(method, "OUTPUT", paths, prototype_props, &mut issues);
        service.issues.extend(issues);
        service.methods.push(ServiceMethod {
            name,
            id,
            input,
            output,
        });
    }
}

fn collect_events(
    _doc: &Document,
    node: Node,
    paths: &[(String, Node)],
    prototype_props: &HashMap<String, SomeIpProps>,
    service: &mut ServiceDef,
) {
    for event in direct_children(node, "EVENTS", "EVENT") {
        let name = short_name(event).to_string();
        let Some(id) = deployment_id(event, paths, "EVENTS", "EVENT") else {
            service
                .issues
                .push(format!("event {} has no SOMEIP event id", name));
            continue;
        };
        let fields = match child_type(event, "TYPE-TREF", paths) {
            Some(ty) => {
                let mut props = element_path(event)
                    .and_then(|path| prototype_props.get(&path).cloned())
                    .unwrap_or_default();
                enrich_meta(event, paths, &mut props);
                vec![ServiceField {
                    name: name.clone(),
                    ty,
                    props,
                }]
            }
            None => {
                service
                    .issues
                    .push(format!("event {} has an unsupported type", name));
                Vec::new()
            }
        };
        service.events.push(ServiceEvent { name, id, fields });
    }
}

fn collect_fields(
    _doc: &Document,
    node: Node,
    paths: &[(String, Node)],
    prototype_props: &HashMap<String, SomeIpProps>,
    service: &mut ServiceDef,
) {
    for field in direct_children(node, "FIELDS", "FIELD") {
        let name = short_name(field).to_string();
        let Some(ty) = child_type(field, "TYPE-TREF", paths) else {
            service
                .issues
                .push(format!("field {name} has an unsupported type"));
            continue;
        };
        let Some(path) = element_path(field) else {
            continue;
        };
        let deployment = paths.iter().find(|(_, candidate)| {
            candidate.tag_name().name() == "SOMEIP-FIELD-DEPLOYMENT"
                && child_text(*candidate, "FIELD-REF").is_some_and(|reference| reference == path)
        });
        let Some((_, deployment)) = deployment else {
            service
                .issues
                .push(format!("field {name} has no SOMEIP field deployment"));
            continue;
        };
        let Some(notifier) = deployment
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == "NOTIFIER")
        else {
            continue;
        };
        let Some(id) = child_text(notifier, "EVENT-ID")
            .and_then(parse_num)
            .and_then(|value| u16::try_from(value).ok())
        else {
            service
                .issues
                .push(format!("field {name} notifier has no event id"));
            continue;
        };
        let mut props = prototype_props.get(&path).cloned().unwrap_or_default();
        enrich_meta(field, paths, &mut props);
        let field_value = ServiceField { name, ty, props };
        service.events.push(ServiceEvent {
            name: field_value.name.clone(),
            id,
            fields: vec![field_value.clone()],
        });
        for (tag, getter) in [("GET", true), ("SET", false)] {
            let Some(access) = deployment
                .children()
                .find(|child| child.is_element() && child.tag_name().name() == tag)
            else {
                continue;
            };
            let Some(id) = child_text(access, "METHOD-ID")
                .and_then(parse_num)
                .and_then(|value| u16::try_from(value).ok())
            else {
                continue;
            };
            if service.methods.iter().any(|method| method.id == id) {
                continue;
            }
            let name = format!(
                "{}_{}",
                if getter { "get" } else { "set" },
                field_value.name
            );
            service.methods.push(ServiceMethod {
                name,
                id,
                input: if getter {
                    Vec::new()
                } else {
                    vec![field_value.clone()]
                },
                output: if getter {
                    vec![field_value.clone()]
                } else {
                    Vec::new()
                },
            });
        }
    }
}

fn eventgroups_of(node: Node, paths: &[(String, Node)]) -> Vec<u16> {
    let mut ids = Vec::new();
    for handler in node
        .descendants()
        .filter(|child| child.has_tag_name("EVENT-HANDLER"))
    {
        if let Some(id) = child_text(handler, "EVENT-GROUP-IDENTIFIER")
            .and_then(parse_num)
            .and_then(|value| u16::try_from(value).ok())
        {
            ids.push(id);
        }
        for reference in handler
            .descendants()
            .filter(|child| child.has_tag_name("CONSUMED-EVENT-GROUP-REF"))
            .filter_map(|child| child.text())
        {
            let tail = reference.rsplit('/').next().unwrap_or(reference);
            if let Some(id) = paths
                .iter()
                .find(|(path, candidate)| {
                    candidate.has_tag_name("CONSUMED-EVENT-GROUP")
                        && (path == reference || path.ends_with(&format!("/{tail}")))
                })
                .and_then(|(_, candidate)| child_text(*candidate, "EVENT-GROUP-IDENTIFIER"))
                .and_then(parse_num)
                .and_then(|value| u16::try_from(value).ok())
            {
                ids.push(id);
            }
        }
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn endpoint_of(node: Node, paths: &[(String, Node)]) -> String {
    let Some(endpoint) = node
        .ancestors()
        .find(|ancestor| ancestor.has_tag_name("APPLICATION-ENDPOINT"))
    else {
        return String::new();
    };
    let port = endpoint
        .descendants()
        .find(|child| child.has_tag_name("PORT-NUMBER"))
        .and_then(|child| child.text())
        .and_then(|text| text.trim().parse::<u16>().ok());
    let ip = endpoint
        .descendants()
        .find(|child| child.has_tag_name("NETWORK-ENDPOINT-REF"))
        .and_then(|child| child.text())
        .and_then(|reference| {
            let tail = reference.rsplit('/').next().unwrap_or(reference);
            paths.iter().find_map(|(path, candidate)| {
                (candidate.has_tag_name("NETWORK-ENDPOINT")
                    && (path == reference || path.ends_with(&format!("/{tail}"))))
                .then(|| {
                    candidate
                        .descendants()
                        .find(|child| child.has_tag_name("IPV-4-ADDRESS"))
                        .and_then(|child| child.text())
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(str::to_string)
                })
                .flatten()
            })
        });
    match (ip, port) {
        (Some(ip), Some(port)) => format!("{ip}:{port}"),
        _ => String::new(),
    }
}

fn collect_instances(paths: &[(String, Node)], service: &mut ServiceDef) {
    for (_, node) in paths {
        if node.tag_name().name() != "PROVIDED-SERVICE-INSTANCE" {
            continue;
        }
        if child_text(*node, "SERVICE-IDENTIFIER")
            .and_then(parse_num)
            .and_then(|value| u16::try_from(value).ok())
            != Some(service.id)
        {
            continue;
        }
        let Some(instance) = child_text(*node, "INSTANCE-IDENTIFIER")
            .and_then(parse_num)
            .and_then(|value| u16::try_from(value).ok())
        else {
            continue;
        };
        let major = child_text(*node, "MAJOR-VERSION")
            .and_then(parse_num)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(0);
        let minor = child_text(*node, "MINOR-VERSION")
            .and_then(parse_num)
            .unwrap_or(0);
        let mut eventgroups = Vec::new();
        for handler in node
            .descendants()
            .filter(|child| child.is_element() && child.tag_name().name() == "EVENT-HANDLER")
        {
            if let Some(id) = child_text(handler, "EVENT-GROUP-IDENTIFIER")
                .and_then(parse_num)
                .and_then(|value| u16::try_from(value).ok())
            {
                eventgroups.push(id);
            }
        }
        service.instances.push(ServiceInstance {
            instance,
            major,
            minor,
            eventgroups,
            endpoint: endpoint_of(*node, paths),
        });
    }
    service.instances.sort_by_key(|entry| entry.instance);
}

fn arguments(
    method: Node,
    direction: &str,
    paths: &[(String, Node)],
    prototype_props: &HashMap<String, SomeIpProps>,
    issues: &mut Vec<String>,
) -> Vec<ServiceField> {
    let mut fields = Vec::new();
    for arguments in method
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == direction)
        .flat_map(|node| node.children())
        .filter(|child| child.is_element() && child.tag_name().name() == "ARGUMENTS")
    {
        for argument in arguments
            .children()
            .filter(|child| child.is_element() && child.tag_name().name() == "ARGUMENT")
        {
            let name = short_name(argument).to_string();
            let Some(ty) = child_type(argument, "TYPE-TREF", paths) else {
                issues.push(format!("argument {name} has an unsupported type"));
                continue;
            };
            let mut props = element_path(argument)
                .and_then(|path| prototype_props.get(&path).cloned())
                .unwrap_or_default();
            enrich_meta(argument, paths, &mut props);
            fields.push(ServiceField { name, ty, props });
        }
    }
    fields
}

fn deployment_id(item: Node, paths: &[(String, Node)], collection: &str, tag: &str) -> Option<u16> {
    let path = element_path(item)?;
    for (_, candidate) in paths {
        let name = candidate.tag_name().name();
        if !matches!(
            name,
            "SOMEIP-METHOD-DEPLOYMENT" | "SOMEIP-EVENT-DEPLOYMENT" | "SOMEIP-FIELD-DEPLOYMENT"
        ) {
            continue;
        }
        let Some(reference) = child_text(*candidate, &format!("{tag}-REF")) else {
            continue;
        };
        if reference != path && !path.ends_with(&reference) {
            continue;
        }
        let _ = collection;
        return child_text(*candidate, &format!("{tag}-ID"))
            .and_then(parse_num)
            .and_then(|value| u16::try_from(value).ok());
    }
    None
}

fn enrich_meta(node: Node, paths: &[(String, Node)], props: &mut SomeIpProps) {
    let mut candidates = vec![node];
    if let Some(reference) = child_text(node, "TYPE-TREF")
        && let Some((_, target)) = paths.iter().find(|(path, _)| *path == reference)
    {
        candidates.push(*target);
    }
    for candidate in candidates {
        let Some(sw) = candidate
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == "SW-DATA-DEF-PROPS")
        else {
            continue;
        };
        if props.unit.is_empty()
            && let Some(reference) = child_text(sw, "UNIT-REF")
            && let Some((_, unit)) = paths.iter().find(|(path, _)| *path == reference)
        {
            props.unit = child_text(*unit, "DISPLAY-NAME").unwrap_or_else(|| short_name(*unit));
        }
        if props.min.is_none()
            && props.max.is_none()
            && let Some(reference) = child_text(sw, "DATA-CONSTR-REF")
            && let Some((_, constraints)) = paths.iter().find(|(path, _)| *path == reference)
            && let Some(internal) = constraints
                .descendants()
                .find(|child| child.is_element() && child.tag_name().name() == "INTERNAL-CONSTRS")
        {
            props.min = child_text(internal, "LOWER-LIMIT");
            props.max = child_text(internal, "UPPER-LIMIT");
        }
    }
}

fn transformation_props(paths: &[(String, Node)]) -> HashMap<String, SomeIpProps> {
    let mut out = HashMap::new();
    for (path, node) in paths {
        if !matches!(
            node.tag_name().name(),
            "SOMEIP-TRANSFORMATION-PROPS" | "AP-SOMEIP-TRANSFORMATION-PROPS"
        ) {
            continue;
        }
        out.insert(
            path.clone(),
            SomeIpProps {
                string_length_field: child_text(*node, "SIZE-OF-STRING-LENGTH-FIELD")
                    .and_then(parse_num)
                    .map_or(4, |value| value as usize),
                union_length_field: child_text(*node, "SIZE-OF-UNION-LENGTH-FIELD")
                    .and_then(parse_num)
                    .map_or(4, |value| value as usize),
                union_type_selector_field: child_text(*node, "SIZE-OF-UNION-TYPE-SELECTOR-FIELD")
                    .and_then(parse_num)
                    .map_or(4, |value| value as usize),
                array_length_field: child_text(*node, "SIZE-OF-ARRAY-LENGTH-FIELD")
                    .and_then(parse_num)
                    .map(|value| value as usize),
                struct_length_field: child_text(*node, "SIZE-OF-STRUCT-LENGTH-FIELD")
                    .and_then(parse_num)
                    .map(|value| value as usize),
                alignment_bits: child_text(*node, "ALIGNMENT")
                    .and_then(parse_num)
                    .map_or(0, |value| value as usize),
                little_endian: child_text(*node, "BYTE-ORDER").as_deref()
                    == Some("MOST-SIGNIFICANT-BYTE-LAST"),
                unit: String::new(),
                min: None,
                max: None,
            },
        );
    }
    out
}

fn prototype_transformation_props(
    paths: &[(String, Node)],
    props: &HashMap<String, SomeIpProps>,
) -> HashMap<String, SomeIpProps> {
    let mut out = HashMap::new();
    for (_, node) in paths {
        let kind = node.tag_name().name();
        let (prototype_tag, props_tag) = match kind {
            "DATA-PROTOTYPE-TRANSFORMATION-PROPS" => (
                "DATA-PROTOTYPE-IN-PORT-INTERFACE-REF",
                "TRANSFORMATION-PROPS-REF",
            ),
            "SOMEIP-DATA-PROTOTYPE-TRANSFORMATION-PROPS" => (
                "DATA-PROTOTYPE-IN-SERVICE-INTERFACE-REF",
                "SOMEIP-TRANSFORMATION-PROPS-REF",
            ),
            _ => continue,
        };
        let Some(prototype) = child_text(*node, prototype_tag) else {
            continue;
        };
        let Some(reference) = child_text(*node, props_tag) else {
            continue;
        };
        if let Some(value) = props.get(&reference) {
            out.insert(prototype, value.clone());
        }
    }
    out
}

fn child_type(node: Node, tag: &str, paths: &[(String, Node)]) -> Option<ServiceType> {
    let reference = child_text(node, tag)?;
    let target = paths
        .iter()
        .find(|(path, _)| path == &reference || path.ends_with(&reference))
        .map(|(_, node)| *node)?;
    type_of(target, paths)
}

fn type_of(node: Node, paths: &[(String, Node)]) -> Option<ServiceType> {
    match node.tag_name().name() {
        "APPLICATION-ARRAY-DATA-TYPE" => {
            let parsed = (|| {
                // AUTOSAR group: ELEMENT carries TYPE-TREF and
                // MAX-NUMBER-OF-ELEMENTS; a dynamic array has no fixed max.
                let element = node
                    .children()
                    .find(|child| child.is_element() && child.tag_name().name() == "ELEMENT")?;
                let count = child_text(element, "MAX-NUMBER-OF-ELEMENTS")
                    .or_else(|| child_text(node, "MAX-NUMBER-OF-ELEMENTS"))
                    .and_then(parse_num)
                    .and_then(|value| usize::try_from(value).ok())?;
                Some(ServiceType::Array {
                    element: Box::new(child_type(element, "TYPE-TREF", paths)?),
                    count,
                })
            })();
            parsed.or_else(|| mapped_type(node, paths))
        }
        "IMPLEMENTATION-DATA-TYPE" => {
            let category = child_text(node, "CATEGORY");
            if category.as_deref() == Some("STRUCTURE") {
                let mut fields = Vec::new();
                for element in node.descendants().filter(|child| {
                    child.is_element()
                        && child.tag_name().name() == "IMPLEMENTATION-DATA-TYPE-ELEMENT"
                }) {
                    let name = short_name(element).to_string();
                    if let Some(ty) = child_type(element, "TYPE-TREF", paths) {
                        fields.push(ServiceField {
                            name,
                            ty,
                            props: SomeIpProps::default(),
                        });
                    }
                }
                Some(ServiceType::Record(fields))
            } else if category.as_deref() == Some("ARRAY") {
                // IMPLEMENTATION-DATA-TYPE ARRAY: SUB-ELEMENTS/…ELEMENT with
                // ARRAY-SIZE and TYPE-TREF.
                let element = node.descendants().find(|child| {
                    child.is_element()
                        && child.tag_name().name() == "IMPLEMENTATION-DATA-TYPE-ELEMENT"
                })?;
                let count = child_text(element, "ARRAY-SIZE")
                    .and_then(parse_num)
                    .and_then(|value| usize::try_from(value).ok())?;
                Some(ServiceType::Array {
                    element: Box::new(child_type(element, "TYPE-TREF", paths)?),
                    count,
                })
            } else if category.as_deref() == Some("UNION") {
                let fields = node
                    .descendants()
                    .filter(|child| {
                        child.is_element()
                            && child.tag_name().name() == "IMPLEMENTATION-DATA-TYPE-ELEMENT"
                    })
                    .filter_map(|element| {
                        let ty = child_type(element, "TYPE-TREF", paths)?;
                        Some(ServiceField {
                            name: short_name(element),
                            ty,
                            props: SomeIpProps::default(),
                        })
                    })
                    .collect();
                Some(ServiceType::Union(fields))
            } else {
                base_type(node, paths)
            }
        }
        "APPLICATION-RECORD-DATA-TYPE" => {
            let mut fields = Vec::new();
            for element in node
                .children()
                .filter(|child| child.is_element() && child.tag_name().name() == "ELEMENTS")
                .flat_map(|parent| parent.children())
                .filter(|child| {
                    child.is_element() && child.tag_name().name() == "APPLICATION-RECORD-ELEMENT"
                })
            {
                if let Some(ty) = child_type(element, "TYPE-TREF", paths) {
                    fields.push(ServiceField {
                        name: short_name(element),
                        ty,
                        props: SomeIpProps::default(),
                    });
                }
            }
            if fields.is_empty() {
                mapped_type(node, paths)
            } else {
                Some(ServiceType::Record(fields))
            }
        }
        "APPLICATION-PRIMITIVE-DATA-TYPE" => {
            let base = base_type(node, paths).or_else(|| mapped_type(node, paths));
            match (base, enum_labels(node, paths)) {
                (Some(base), Some(labels)) => Some(ServiceType::Enum {
                    base: Box::new(base),
                    labels,
                }),
                (base, _) => base,
            }
        }
        _ => base_type(node, paths),
    }
}

/// Follows DATA-TYPE-MAPPING-SET / DATA-TYPE-MAP from an application type
/// to its implementation type. This is the AUTOSAR typedef path: service
/// arguments usually reference application types, while their byte layout
/// lives on the implementation type.
/// Reads the COMPU-METHOD referenced by a primitive type when it is a
/// TEXTTABLE. The numeric base type still comes from BASE-TYPE-REF or the
/// DATA-TYPE-MAP; labels only decorate that value.
fn enum_labels(node: Node, paths: &[(String, Node)]) -> Option<Vec<(i64, String)>> {
    let reference = node
        .descendants()
        .filter(|child| child.is_element())
        .find_map(|child| {
            (child.tag_name().name() == "COMPU-METHOD-REF")
                .then(|| child.text().unwrap_or("").to_string())
        })?;
    let compu = paths
        .iter()
        .find(|(path, _)| path == &reference)
        .map(|(_, node)| *node)?;
    if child_text(compu, "CATEGORY").as_deref() != Some("TEXTTABLE") {
        return None;
    }
    let mut labels = Vec::new();
    for scale in compu
        .descendants()
        .filter(|child| child.is_element() && child.tag_name().name() == "COMPU-SCALE")
    {
        let value = child_text(scale, "LOWER-LIMIT")
            .and_then(parse_num)
            .map(i64::from)?;
        let text = scale
            .descendants()
            .filter(|child| child.is_element() && child.tag_name().name() == "VT")
            .find_map(|child| child.text())?;
        labels.push((value, text.trim().to_string()));
    }
    (!labels.is_empty()).then_some(labels)
}

fn mapped_type(node: Node, paths: &[(String, Node)]) -> Option<ServiceType> {
    let path = element_path(node)?;
    for (_, candidate) in paths {
        if candidate.tag_name().name() != "DATA-TYPE-MAP" {
            continue;
        }
        let Some(application) = child_text(*candidate, "APPLICATION-DATA-TYPE-REF") else {
            continue;
        };
        if application != path {
            continue;
        }
        let Some(implementation) = child_text(*candidate, "IMPLEMENTATION-DATA-TYPE-REF") else {
            continue;
        };
        let target = paths
            .iter()
            .find(|(candidate, _)| candidate == &implementation)
            .map(|(_, node)| *node)?;
        return type_of(target, paths);
    }
    None
}

fn align_after(props: &SomeIpProps, at: &mut usize) {
    let bits = props.alignment_bits;
    if bits == 0 {
        return;
    }
    let bytes = bits.div_ceil(8).max(1);
    *at = at.div_ceil(bytes) * bytes;
}

fn read_length(bytes: &[u8]) -> Option<usize> {
    let mut value = 0usize;
    for byte in bytes {
        value = value.checked_mul(256)?.checked_add(usize::from(*byte))?;
    }
    Some(value)
}

fn base_type(node: Node, _paths: &[(String, Node)]) -> Option<ServiceType> {
    let reference = node
        .descendants()
        .filter(|child| child.is_element() && child.tag_name().name() == "BASE-TYPE-REF")
        .find_map(|child| child.text())?;
    match reference.rsplit('/').next()?.trim() {
        "uint8" | "uint8_t" => Some(ServiceType::U8),
        "uint16" | "uint16_t" => Some(ServiceType::U16),
        "uint32" | "uint32_t" => Some(ServiceType::U32),
        "uint64" | "uint64_t" => Some(ServiceType::U64),
        "sint8" | "sint8_t" => Some(ServiceType::I8),
        "sint16" | "sint16_t" => Some(ServiceType::I16),
        "sint32" | "sint32_t" => Some(ServiceType::I32),
        "sint64" | "sint64_t" => Some(ServiceType::I64),
        "float32" | "float" => Some(ServiceType::F32),
        "float64" | "double" => Some(ServiceType::F64),
        "boolean" | "bool" => Some(ServiceType::Bool),
        "string" | "utf8" | "utf-8" => Some(ServiceType::String),
        _ => None,
    }
}

fn direct_children<'a, 'input>(
    node: Node<'a, 'input>,
    container: &str,
    tag: &str,
) -> Vec<Node<'a, 'input>> {
    node.children()
        .filter(|child| child.is_element() && child.tag_name().name() == container)
        .flat_map(|parent| parent.children())
        .filter(|child| child.is_element() && child.tag_name().name() == tag)
        .collect()
}

fn child_text(node: Node, tag: &str) -> Option<String> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == tag)
        .and_then(|child| child.text())
        .map(|text| text.trim().to_string())
}

fn short_name(node: Node) -> String {
    child_text(node, "SHORT-NAME").unwrap_or_default()
}

fn parse_num(text: String) -> Option<u32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        text.parse().ok()
    }
}

fn element_path(node: Node) -> Option<String> {
    let mut parts = Vec::new();
    let mut current = Some(node);
    while let Some(element) = current {
        if element.is_element()
            && let Some(name) = child_text(element, "SHORT-NAME")
        {
            parts.push(name);
        }
        current = element.parent();
    }
    parts.reverse();
    Some(format!("/{}", parts.join("/")))
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceValue {
    pub name: String,
    pub text: String,
    /// Numeric value when the decoded type has one (enum labels keep it).
    pub numeric: Option<f64>,
}

/// Decodes packed network-order fields. Records are flattened with
/// `parent.child` names. Returns `None` when the payload is shorter than the
/// declared layout; the caller then keeps the raw bytes instead of showing a
/// truncated value.
pub fn decode_values(fields: &[ServiceField], payload: &[u8]) -> Option<Vec<ServiceValue>> {
    let mut at = 0usize;
    let mut out = Vec::new();
    for field in fields {
        decode_field(
            &field.name,
            &field.ty,
            &field.props,
            payload,
            &mut at,
            &mut out,
        )?;
    }
    Some(out)
}

/// Encodes physical values named like the decoded fields (`wheel[0]`,
/// `Details.Speed`) into a SOME/IP payload using the ARXML layout. Missing
/// values default to 0 so a Panel/Trigger can change one field without
/// spelling out the whole record.
pub fn encode_values(
    fields: &[ServiceField],
    values: &std::collections::BTreeMap<String, f64>,
) -> Result<Vec<u8>, String> {
    encode_values_with_text(fields, values, &std::collections::BTreeMap::new())
}

/// Physical encode with a text side table for `string` fields. Numeric values
/// still drive primitives, arrays and records; `$selector` picks a union
/// variant by index.
pub fn encode_values_with_text(
    fields: &[ServiceField],
    values: &std::collections::BTreeMap<String, f64>,
    texts: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for field in fields {
        encode_field(
            &field.name,
            &field.ty,
            &field.props,
            values,
            texts,
            &mut out,
        )?;
    }
    Ok(out)
}

fn encode_field(
    name: &str,
    ty: &ServiceType,
    props: &SomeIpProps,
    values: &std::collections::BTreeMap<String, f64>,
    texts: &std::collections::BTreeMap<String, String>,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let number = |name: &str| values.get(name).copied().unwrap_or(0.0);
    let primitive = |out: &mut Vec<u8>, bytes: &[u8]| {
        if props.little_endian {
            out.extend(bytes.iter().rev());
        } else {
            out.extend_from_slice(bytes);
        }
    };
    match ty {
        ServiceType::U8 => out.push(number(name) as u8),
        ServiceType::I8 => out.push(number(name) as i8 as u8),
        ServiceType::U16 => primitive(out, &(number(name) as u16).to_be_bytes()),
        ServiceType::U32 => primitive(out, &(number(name) as u32).to_be_bytes()),
        ServiceType::U64 => primitive(out, &(number(name) as u64).to_be_bytes()),
        ServiceType::I16 => primitive(out, &(number(name) as i16).to_be_bytes()),
        ServiceType::I32 => primitive(out, &(number(name) as i32).to_be_bytes()),
        ServiceType::I64 => primitive(out, &(number(name) as i64).to_be_bytes()),
        ServiceType::F32 => primitive(out, &(number(name) as f32).to_be_bytes()),
        ServiceType::F64 => primitive(out, &number(name).to_be_bytes()),
        ServiceType::Bool => out.push(u8::from(number(name) != 0.0)),
        ServiceType::Record(fields) => {
            let mut body = Vec::new();
            for field in fields {
                encode_field(
                    &format!("{name}.{}", field.name),
                    &field.ty,
                    &field.props,
                    values,
                    texts,
                    &mut body,
                )?;
            }
            if let Some(width) = props.struct_length_field {
                width_bytes(out, width, body.len())?;
            }
            out.extend_from_slice(&body);
        }
        ServiceType::Array { element, count } => {
            let count = values
                .get(&format!("{name}.$count"))
                .map(|value| (*value).max(0.0) as usize)
                .unwrap_or(*count);
            if count > 4096 {
                return Err(format!("{name}: array count {count} is too large"));
            }
            let mut body = Vec::new();
            for index in 0..count {
                encode_field(
                    &format!("{name}[{index}]"),
                    element,
                    props,
                    values,
                    texts,
                    &mut body,
                )?;
            }
            if let Some(width) = props.array_length_field {
                width_bytes(out, width, body.len())?;
            }
            out.extend_from_slice(&body);
        }
        ServiceType::Enum { base, .. } => encode_field(name, base, props, values, texts, out)?,
        ServiceType::String => {
            let text = texts.get(name).cloned().unwrap_or_default();
            let bytes = text.as_bytes();
            width_bytes(out, props.string_length_field, bytes.len())?;
            out.extend_from_slice(bytes);
        }
        ServiceType::Union(variants) => {
            let selected = number(&format!("{name}.$selector")) as usize;
            let Some(variant) = variants.get(selected) else {
                return Err(format!(
                    "{name}: union selector {selected} has no variant (0..{})",
                    variants.len().saturating_sub(1)
                ));
            };
            let mut body = Vec::new();
            encode_field(
                &format!("{name}.{}", variant.name),
                &variant.ty,
                &variant.props,
                values,
                texts,
                &mut body,
            )?;
            width_bytes(out, props.union_length_field, body.len())?;
            width_bytes(out, props.union_type_selector_field, selected)?;
            out.extend_from_slice(&body);
        }
    }
    Ok(())
}

fn width_bytes(out: &mut Vec<u8>, width: usize, value: usize) -> Result<(), String> {
    let width = width.clamp(1, 8);
    if width < 8 && value >= (1usize << (width * 8)) {
        return Err(format!("length {value} does not fit in {width} bytes"));
    }
    out.extend_from_slice(&value.to_be_bytes()[8 - width..]);
    Ok(())
}

fn decode_field(
    name: &str,
    ty: &ServiceType,
    props: &SomeIpProps,
    payload: &[u8],
    at: &mut usize,
    out: &mut Vec<ServiceValue>,
) -> Option<()> {
    let take = |size: usize, at: &mut usize| -> Option<&[u8]> {
        let bytes = payload.get(*at..*at + size)?;
        *at += size;
        Some(bytes)
    };
    let little = props.little_endian;
    let text = match ty {
        ServiceType::U8 => take(1, at)?[0].to_string(),
        ServiceType::U16 => {
            let bytes = take(2, at)?.try_into().ok()?;
            (if little {
                u16::from_le_bytes(bytes)
            } else {
                u16::from_be_bytes(bytes)
            })
            .to_string()
        }
        ServiceType::U32 => {
            let bytes = take(4, at)?.try_into().ok()?;
            (if little {
                u32::from_le_bytes(bytes)
            } else {
                u32::from_be_bytes(bytes)
            })
            .to_string()
        }
        ServiceType::U64 => {
            let bytes = take(8, at)?.try_into().ok()?;
            (if little {
                u64::from_le_bytes(bytes)
            } else {
                u64::from_be_bytes(bytes)
            })
            .to_string()
        }
        ServiceType::I8 => i8::from_be_bytes(take(1, at)?.try_into().ok()?).to_string(),
        ServiceType::I16 => {
            let bytes = take(2, at)?.try_into().ok()?;
            (if little {
                i16::from_le_bytes(bytes)
            } else {
                i16::from_be_bytes(bytes)
            })
            .to_string()
        }
        ServiceType::I32 => {
            let bytes = take(4, at)?.try_into().ok()?;
            (if little {
                i32::from_le_bytes(bytes)
            } else {
                i32::from_be_bytes(bytes)
            })
            .to_string()
        }
        ServiceType::I64 => {
            let bytes = take(8, at)?.try_into().ok()?;
            (if little {
                i64::from_le_bytes(bytes)
            } else {
                i64::from_be_bytes(bytes)
            })
            .to_string()
        }
        ServiceType::F32 => {
            let bytes = take(4, at)?.try_into().ok()?;
            (if little {
                f32::from_le_bytes(bytes)
            } else {
                f32::from_be_bytes(bytes)
            })
            .to_string()
        }
        ServiceType::F64 => {
            let bytes = take(8, at)?.try_into().ok()?;
            (if little {
                f64::from_le_bytes(bytes)
            } else {
                f64::from_be_bytes(bytes)
            })
            .to_string()
        }
        ServiceType::Bool => (take(1, at)?[0] != 0).to_string(),
        ServiceType::Record(nested) => {
            let prefix = props.struct_length_field;
            let length = match prefix {
                Some(width) => Some(read_length(take(width.clamp(1, 8), at)?)?),
                None => None,
            };
            let start = *at;
            for field in nested {
                decode_field(
                    &format!("{name}.{}", field.name),
                    &field.ty,
                    &field.props,
                    payload,
                    at,
                    out,
                )?;
            }
            if let Some(length) = length
                && *at - start != length
            {
                return None;
            }
            align_after(props, at);
            return Some(());
        }
        ServiceType::Array { element, count } => {
            let count = if let Some(width) = props.array_length_field {
                let length = read_length(take(width.clamp(1, 8), at)?)?;
                let size = element.fixed_size()?;
                if size == 0 {
                    return None;
                }
                length / size
            } else {
                *count
            };
            for index in 0..count {
                decode_field(
                    &format!("{name}[{index}]"),
                    element,
                    props,
                    payload,
                    at,
                    out,
                )?;
            }
            align_after(props, at);
            return Some(());
        }
        ServiceType::Enum { base, labels } => {
            let before = out.len();
            decode_field(name, base, props, payload, at, out)?;
            if let Ok(value) = out[before].text.parse::<i64>()
                && let Some((_, label)) = labels.iter().find(|(key, _)| *key == value)
            {
                out[before].numeric = Some(value as f64);
                out[before].text = label.clone();
            }
            return Some(());
        }
        ServiceType::String => {
            let width = props.string_length_field.clamp(1, 8);
            let length = read_length(take(width, at)?)?;
            let bytes = take(length, at)?;
            let end = bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(bytes.len());
            String::from_utf8_lossy(&bytes[..end]).into_owned()
        }
        ServiceType::Union(variants) => {
            let length_width = props.union_length_field.clamp(1, 8);
            let selector_width = props.union_type_selector_field.clamp(1, 8);
            let length = read_length(take(length_width, at)?)?;
            let selector = read_length(take(selector_width, at)?)?;
            let value = take(length.min(payload.len().saturating_sub(*at)), at)?;
            let names = variants
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>()
                .join("/");
            format!(
                "selector={selector} {}{}",
                value
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                if names.is_empty() {
                    String::new()
                } else {
                    format!(" ({names})")
                }
            )
        }
    };
    if matches!(ty, ServiceType::String | ServiceType::Union(_)) {
        align_after(props, at);
    }
    out.push(ServiceValue {
        name: name.to_string(),
        text,
        numeric: None,
    });
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"<?xml version="1.0"?>
<AUTOSAR>
 <AR-PACKAGES><AR-PACKAGE><SHORT-NAME>Pkg</SHORT-NAME><ELEMENTS>
  <SERVICE-INTERFACE><SHORT-NAME>SpeedService</SHORT-NAME>
   <METHODS><METHOD><SHORT-NAME>GetSpeed</SHORT-NAME>
    <INPUT><ARGUMENTS>
     <ARGUMENT><SHORT-NAME>Request</SHORT-NAME>
      <TYPE-TREF DEST="APPLICATION-PRIMITIVE-DATA-TYPE">/Pkg/uint16</TYPE-TREF>
      <SW-DATA-DEF-PROPS>
       <UNIT-REF DEST="UNIT">/Pkg/kmh</UNIT-REF>
       <DATA-CONSTR-REF DEST="DATA-CONSTR">/Pkg/SpeedRange</DATA-CONSTR-REF>
      </SW-DATA-DEF-PROPS>
     </ARGUMENT>
     <ARGUMENT><SHORT-NAME>Wheels</SHORT-NAME>
      <TYPE-TREF DEST="APPLICATION-ARRAY-DATA-TYPE">/Pkg/WheelArray</TYPE-TREF>
     </ARGUMENT>
     <ARGUMENT><SHORT-NAME>Details</SHORT-NAME>
      <TYPE-TREF DEST="APPLICATION-RECORD-DATA-TYPE">/Pkg/WheelRecord</TYPE-TREF>
     </ARGUMENT>
     <ARGUMENT><SHORT-NAME>Mapped</SHORT-NAME>
      <TYPE-TREF DEST="APPLICATION-PRIMITIVE-DATA-TYPE">/Pkg/WheelSpeed</TYPE-TREF>
     </ARGUMENT>
    </ARGUMENTS></INPUT>
    <OUTPUT><ARGUMENTS>
     <ARGUMENT><SHORT-NAME>Speed</SHORT-NAME>
      <TYPE-TREF DEST="APPLICATION-PRIMITIVE-DATA-TYPE">/Pkg/uint32</TYPE-TREF>
     </ARGUMENT>
    </ARGUMENTS></OUTPUT>
   </METHOD></METHODS>
   <EVENTS><EVENT><SHORT-NAME>SpeedEvent</SHORT-NAME>
    <TYPE-TREF DEST="APPLICATION-PRIMITIVE-DATA-TYPE">/Pkg/uint16</TYPE-TREF>
   </EVENT></EVENTS>
   <FIELDS><FIELD><SHORT-NAME>Temperature</SHORT-NAME>
    <TYPE-TREF DEST="APPLICATION-PRIMITIVE-DATA-TYPE">/Pkg/uint16</TYPE-TREF>
   </FIELD></FIELDS>
  </SERVICE-INTERFACE>
  <UNIT><SHORT-NAME>kmh</SHORT-NAME><DISPLAY-NAME>km/h</DISPLAY-NAME></UNIT>
  <DATA-CONSTR><SHORT-NAME>SpeedRange</SHORT-NAME>
   <DATA-CONSTR-RULES><DATA-CONSTR-RULE><INTERNAL-CONSTRS>
    <LOWER-LIMIT>0</LOWER-LIMIT><UPPER-LIMIT>250</UPPER-LIMIT>
   </INTERNAL-CONSTRS></DATA-CONSTR-RULE></DATA-CONSTR-RULES>
  </DATA-CONSTR>
  <APPLICATION-PRIMITIVE-DATA-TYPE><SHORT-NAME>uint16</SHORT-NAME>
   <SW-DATA-DEF-PROPS><SW-DATA-DEF-PROPS-VARIANTS><SW-DATA-DEF-PROPS-CONDITIONAL>
    <BASE-TYPE-REF DEST="SW-BASE-TYPE">/Pkg/uint16</BASE-TYPE-REF>
   </SW-DATA-DEF-PROPS-CONDITIONAL></SW-DATA-DEF-PROPS-VARIANTS></SW-DATA-DEF-PROPS>
  </APPLICATION-PRIMITIVE-DATA-TYPE>
  <APPLICATION-PRIMITIVE-DATA-TYPE><SHORT-NAME>uint32</SHORT-NAME>
   <SW-DATA-DEF-PROPS><SW-DATA-DEF-PROPS-VARIANTS><SW-DATA-DEF-PROPS-CONDITIONAL>
    <BASE-TYPE-REF DEST="SW-BASE-TYPE">/Pkg/uint32</BASE-TYPE-REF>
   </SW-DATA-DEF-PROPS-CONDITIONAL></SW-DATA-DEF-PROPS-VARIANTS></SW-DATA-DEF-PROPS>
  </APPLICATION-PRIMITIVE-DATA-TYPE>
  <APPLICATION-ARRAY-DATA-TYPE><SHORT-NAME>WheelArray</SHORT-NAME>
   <ELEMENT><SHORT-NAME>Wheel</SHORT-NAME>
    <TYPE-TREF DEST="APPLICATION-PRIMITIVE-DATA-TYPE">/Pkg/uint16</TYPE-TREF>
    <MAX-NUMBER-OF-ELEMENTS>4</MAX-NUMBER-OF-ELEMENTS>
   </ELEMENT>
  </APPLICATION-ARRAY-DATA-TYPE>
  <APPLICATION-RECORD-DATA-TYPE><SHORT-NAME>WheelRecord</SHORT-NAME>
   <ELEMENTS>
    <APPLICATION-RECORD-ELEMENT><SHORT-NAME>Speed</SHORT-NAME>
     <TYPE-TREF DEST="APPLICATION-PRIMITIVE-DATA-TYPE">/Pkg/uint16</TYPE-TREF>
    </APPLICATION-RECORD-ELEMENT>
    <APPLICATION-RECORD-ELEMENT><SHORT-NAME>Rpm</SHORT-NAME>
     <TYPE-TREF DEST="APPLICATION-PRIMITIVE-DATA-TYPE">/Pkg/uint32</TYPE-TREF>
    </APPLICATION-RECORD-ELEMENT>
   </ELEMENTS>
  </APPLICATION-RECORD-DATA-TYPE>
  <APPLICATION-PRIMITIVE-DATA-TYPE><SHORT-NAME>WheelSpeed</SHORT-NAME>
   <SW-DATA-DEF-PROPS><SW-DATA-DEF-PROPS-VARIANTS><SW-DATA-DEF-PROPS-CONDITIONAL>
    <COMPU-METHOD-REF DEST="COMPU-METHOD">/Pkg/WheelSpeedCompu</COMPU-METHOD-REF>
   </SW-DATA-DEF-PROPS-CONDITIONAL></SW-DATA-DEF-PROPS-VARIANTS></SW-DATA-DEF-PROPS>
  </APPLICATION-PRIMITIVE-DATA-TYPE>
  <COMPU-METHOD><SHORT-NAME>WheelSpeedCompu</SHORT-NAME>
   <CATEGORY>TEXTTABLE</CATEGORY>
   <COMPU-INTERNAL-TO-PHYS><COMPU-SCALES>
    <COMPU-SCALE><LOWER-LIMIT>1</LOWER-LIMIT>
     <COMPU-CONST><VT>Idle</VT></COMPU-CONST></COMPU-SCALE>
    <COMPU-SCALE><LOWER-LIMIT>2</LOWER-LIMIT>
     <COMPU-CONST><VT>Moving</VT></COMPU-CONST></COMPU-SCALE>
   </COMPU-SCALES></COMPU-INTERNAL-TO-PHYS>
  </COMPU-METHOD>
  <IMPLEMENTATION-DATA-TYPE><SHORT-NAME>uint16Impl</SHORT-NAME>
   <CATEGORY>VALUE</CATEGORY>
   <SW-DATA-DEF-PROPS><SW-DATA-DEF-PROPS-VARIANTS><SW-DATA-DEF-PROPS-CONDITIONAL>
    <BASE-TYPE-REF DEST="SW-BASE-TYPE">/Pkg/uint16</BASE-TYPE-REF>
   </SW-DATA-DEF-PROPS-CONDITIONAL></SW-DATA-DEF-PROPS-VARIANTS></SW-DATA-DEF-PROPS>
  </IMPLEMENTATION-DATA-TYPE>
  <DATA-TYPE-MAPPING-SET><SHORT-NAME>ServiceTypes</SHORT-NAME>
   <DATA-TYPE-MAPS><DATA-TYPE-MAP>
    <APPLICATION-DATA-TYPE-REF DEST="APPLICATION-PRIMITIVE-DATA-TYPE">/Pkg/WheelSpeed</APPLICATION-DATA-TYPE-REF>
    <IMPLEMENTATION-DATA-TYPE-REF DEST="IMPLEMENTATION-DATA-TYPE">/Pkg/uint16Impl</IMPLEMENTATION-DATA-TYPE-REF>
   </DATA-TYPE-MAP></DATA-TYPE-MAPS>
  </DATA-TYPE-MAPPING-SET>
  <SOMEIP-TRANSFORMATION-PROPS><SHORT-NAME>String2</SHORT-NAME>
   <SIZE-OF-STRING-LENGTH-FIELD>2</SIZE-OF-STRING-LENGTH-FIELD>
  </SOMEIP-TRANSFORMATION-PROPS>
  <DATA-PROTOTYPE-TRANSFORMATION-PROPS>
   <SHORT-NAME>RequestProps</SHORT-NAME>
   <DATA-PROTOTYPE-IN-PORT-INTERFACE-REF DEST="VARIABLE-DATA-PROTOTYPE">/Pkg/SpeedService/GetSpeed/Request</DATA-PROTOTYPE-IN-PORT-INTERFACE-REF>
   <TRANSFORMATION-PROPS-REF DEST="SOMEIP-TRANSFORMATION-PROPS">/Pkg/String2</TRANSFORMATION-PROPS-REF>
  </DATA-PROTOTYPE-TRANSFORMATION-PROPS>
  <PROVIDED-SERVICE-INSTANCE><SHORT-NAME>SpeedInstance</SHORT-NAME>
   <SERVICE-IDENTIFIER>4660</SERVICE-IDENTIFIER>
   <INSTANCE-IDENTIFIER>1</INSTANCE-IDENTIFIER>
   <MAJOR-VERSION>1</MAJOR-VERSION>
   <MINOR-VERSION>2</MINOR-VERSION>
   <EVENT-HANDLERS><EVENT-HANDLER>
    <EVENT-GROUP-IDENTIFIER>3</EVENT-GROUP-IDENTIFIER>
   </EVENT-HANDLER></EVENT-HANDLERS>
  </PROVIDED-SERVICE-INSTANCE>
  <SOMEIP-SERVICE-INTERFACE-DEPLOYMENT><SHORT-NAME>SpeedDeploy</SHORT-NAME>
   <SERVICE-INTERFACE-REF DEST="SERVICE-INTERFACE">/Pkg/SpeedService</SERVICE-INTERFACE-REF>
   <SERVICE-INTERFACE-ID>4660</SERVICE-INTERFACE-ID>
   <METHOD-DEPLOYMENTS><SOMEIP-METHOD-DEPLOYMENT>
    <METHOD-REF DEST="METHOD">/Pkg/SpeedService/GetSpeed</METHOD-REF>
    <METHOD-ID>1</METHOD-ID>
   </SOMEIP-METHOD-DEPLOYMENT></METHOD-DEPLOYMENTS>
   <EVENT-DEPLOYMENTS><SOMEIP-EVENT-DEPLOYMENT>
    <EVENT-REF DEST="EVENT">/Pkg/SpeedService/SpeedEvent</EVENT-REF>
    <EVENT-ID>32769</EVENT-ID>
   </SOMEIP-EVENT-DEPLOYMENT></EVENT-DEPLOYMENTS>
   <FIELD-DEPLOYMENTS><SOMEIP-FIELD-DEPLOYMENT>
    <FIELD-REF DEST="FIELD">/Pkg/SpeedService/Temperature</FIELD-REF>
    <NOTIFIER><EVENT-ID>32770</EVENT-ID></NOTIFIER>
    <GET><METHOD-ID>32771</METHOD-ID></GET>
    <SET><METHOD-ID>32772</METHOD-ID></SET>
   </SOMEIP-FIELD-DEPLOYMENT></FIELD-DEPLOYMENTS>
  </SOMEIP-SERVICE-INTERFACE-DEPLOYMENT>
 </ELEMENTS></AR-PACKAGE></AR-PACKAGES>
</AUTOSAR>"#;

    #[test]
    fn reads_service_ids_and_method_event_ids_from_deployments() {
        let catalog = scan(DOC).expect("catalog");
        assert_eq!(catalog.services.len(), 1);
        let service = &catalog.services[0];
        assert_eq!(service.id, 0x1234);
        assert_eq!(service.methods[0].id, 1);
        assert_eq!(service.events[0].id, 0x8001);
        assert_eq!(service.events[1].name, "Temperature");
        assert_eq!(service.events[1].id, 0x8002);
        assert!(service.methods.iter().any(|method| method.id == 0x8003));
        assert!(service.methods.iter().any(|method| method.id == 0x8004));
        assert_eq!(service.instances.len(), 1);
        assert_eq!(service.instances[0].instance, 1);
        assert_eq!(service.instances[0].major, 1);
        assert_eq!(service.instances[0].minor, 2);
        assert_eq!(service.instances[0].eventgroups, [3]);
        assert_eq!(service.methods[0].input[0].props.string_length_field, 2);
        assert_eq!(service.methods[0].input[0].props.unit, "km/h");
        assert_eq!(service.methods[0].input[0].props.min.as_deref(), Some("0"));
        assert_eq!(
            service.methods[0].input[0].props.max.as_deref(),
            Some("250")
        );
        assert_eq!(
            service.methods[0].input[1].ty,
            ServiceType::Array {
                element: Box::new(ServiceType::U16),
                count: 4,
            }
        );
        assert!(matches!(
            service.methods[0].input[2].ty,
            ServiceType::Record(ref fields) if fields.len() == 2
        ));
        assert!(matches!(
            &service.methods[0].input[3].ty,
            ServiceType::Enum { labels, .. }
                if labels == &vec![(1, "Idle".to_string()), (2, "Moving".to_string())]
        ));
    }

    #[test]
    fn encodes_named_values_from_arxml_layout() {
        let fields = vec![
            ServiceField {
                name: "speed".into(),
                ty: ServiceType::U16,
                props: SomeIpProps::default(),
            },
            ServiceField {
                name: "wheel".into(),
                ty: ServiceType::Array {
                    element: Box::new(ServiceType::U8),
                    count: 2,
                },
                props: SomeIpProps::default(),
            },
        ];
        let values = std::collections::BTreeMap::from([
            ("speed".to_string(), 258.0),
            ("wheel[0]".to_string(), 7.0),
            ("wheel[1]".to_string(), 9.0),
        ]);
        let bytes = encode_values(&fields, &values).unwrap();
        assert_eq!(bytes, [0x01, 0x02, 7, 9]);
        assert_eq!(decode_values(&fields, &bytes).unwrap()[0].text, "258");
    }

    #[test]
    fn decodes_array_and_struct_length_fields() {
        let arrays = vec![ServiceField {
            name: "value".into(),
            ty: ServiceType::Array {
                element: Box::new(ServiceType::U16),
                count: 2,
            },
            props: SomeIpProps {
                array_length_field: Some(1),
                ..SomeIpProps::default()
            },
        }];
        let values = decode_values(&arrays, &[4, 0, 1, 0, 2]).unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[1].text, "2");

        let records = vec![ServiceField {
            name: "record".into(),
            ty: ServiceType::Record(vec![
                ServiceField {
                    name: "a".into(),
                    ty: ServiceType::U8,
                    props: SomeIpProps::default(),
                },
                ServiceField {
                    name: "b".into(),
                    ty: ServiceType::U8,
                    props: SomeIpProps::default(),
                },
            ]),
            props: SomeIpProps {
                struct_length_field: Some(1),
                ..SomeIpProps::default()
            },
        }];
        let values = decode_values(&records, &[2, 5, 6]).unwrap();
        assert_eq!(values[0].text, "5");
        assert_eq!(values[1].text, "6");
    }

    #[test]
    fn dynamic_array_count_overrides_the_declared_count() {
        let fields = vec![ServiceField {
            name: "wheel".into(),
            ty: ServiceType::Array {
                element: Box::new(ServiceType::U8),
                count: 0,
            },
            props: SomeIpProps {
                array_length_field: Some(2),
                ..SomeIpProps::default()
            },
        }];
        let mut values = std::collections::BTreeMap::new();
        values.insert("wheel.$count".to_string(), 2.0);
        values.insert("wheel[0]".to_string(), 1.0);
        values.insert("wheel[1]".to_string(), 2.0);
        assert_eq!(
            encode_values_with_text(&fields, &values, &Default::default()).unwrap(),
            [0, 2, 1, 2]
        );
    }

    #[test]
    fn encodes_text_strings_and_selected_union_variants() {
        let string = vec![ServiceField {
            name: "name".into(),
            ty: ServiceType::String,
            props: SomeIpProps {
                string_length_field: 2,
                ..SomeIpProps::default()
            },
        }];
        let mut texts = std::collections::BTreeMap::new();
        texts.insert("name".to_string(), "abc".to_string());
        assert_eq!(
            encode_values_with_text(&string, &Default::default(), &texts).unwrap(),
            [0, 3, b'a', b'b', b'c']
        );

        let union = vec![ServiceField {
            name: "value".into(),
            ty: ServiceType::Union(vec![
                ServiceField {
                    name: "U16".into(),
                    ty: ServiceType::U16,
                    props: SomeIpProps::default(),
                },
                ServiceField {
                    name: "U8".into(),
                    ty: ServiceType::U8,
                    props: SomeIpProps::default(),
                },
            ]),
            props: SomeIpProps::default(),
        }];
        let mut values = std::collections::BTreeMap::new();
        values.insert("value.$selector".to_string(), 1.0);
        values.insert("value.U8".to_string(), 7.0);
        assert_eq!(
            encode_values_with_text(&union, &values, &Default::default()).unwrap(),
            [0, 0, 0, 1, 0, 0, 0, 1, 7]
        );
    }

    #[test]
    fn decodes_default_strings_and_unions() {
        let strings = vec![ServiceField {
            name: "name".into(),
            ty: ServiceType::String,
            props: SomeIpProps {
                string_length_field: 2,
                ..SomeIpProps::default()
            },
        }];
        assert_eq!(
            decode_values(&strings, &[0, 3, b'a', b'b', b'c']).unwrap()[0].text,
            "abc"
        );

        let unions = vec![ServiceField {
            name: "value".into(),
            ty: ServiceType::Union(vec![ServiceField {
                name: "U16".into(),
                ty: ServiceType::U16,
                props: SomeIpProps::default(),
            }]),
            props: SomeIpProps::default(),
        }];
        let values = decode_values(&unions, &[0, 0, 0, 2, 0, 0, 0, 1, 0x12, 0x34]).unwrap();
        assert!(values[0].text.contains("selector=1"));
        assert!(values[0].text.contains("12 34"));
    }

    #[test]
    fn decodes_enum_labels() {
        let fields = vec![ServiceField {
            name: "state".into(),
            ty: ServiceType::Enum {
                base: Box::new(ServiceType::U8),
                labels: vec![(1, "Idle".into()), (2, "Moving".into())],
            },
            props: SomeIpProps::default(),
        }];
        let values = decode_values(&fields, &[2]).unwrap();
        assert_eq!(values[0].text, "Moving");
        assert_eq!(values[0].numeric, Some(2.0));
    }

    #[test]
    fn decodes_fixed_arrays() {
        let fields = vec![ServiceField {
            name: "wheel".into(),
            ty: ServiceType::Array {
                element: Box::new(ServiceType::U16),
                count: 2,
            },
            props: SomeIpProps::default(),
        }];
        let values = decode_values(&fields, &[0, 1, 0, 2]).unwrap();
        assert_eq!(values[0].name, "wheel[0]");
        assert_eq!(values[0].text, "1");
        assert_eq!(values[1].name, "wheel[1]");
        assert_eq!(values[1].text, "2");
    }
    #[test]
    fn decodes_packed_network_order_fields() {
        let fields = vec![
            ServiceField {
                name: "speed".into(),
                ty: ServiceType::U16,
                props: SomeIpProps::default(),
            },
            ServiceField {
                name: "rpm".into(),
                ty: ServiceType::U32,
                props: SomeIpProps::default(),
            },
        ];
        let values = decode_values(&fields, &[0x01, 0x02, 0x00, 0x00, 0x0f, 0xa0]).unwrap();
        assert_eq!(values[0].text, "258");
        assert_eq!(values[1].text, "4000");
        assert!(decode_values(&fields, &[0, 1]).is_none());

        let little = vec![ServiceField {
            name: "le".into(),
            ty: ServiceType::U16,
            props: SomeIpProps {
                little_endian: true,
                ..SomeIpProps::default()
            },
        }];
        assert_eq!(decode_values(&little, &[1, 0]).unwrap()[0].text, "1");
    }

    #[test]
    fn provided_service_instance_keeps_endpoint() {
        let text = r#"<?xml version="1.0"?>
<AUTOSAR xmlns="http://autosar.org/schema/r4.0">
 <AR-PACKAGES><AR-PACKAGE><SHORT-NAME>P</SHORT-NAME><ELEMENTS>
  <NETWORK-ENDPOINT><SHORT-NAME>Nep</SHORT-NAME>
   <IPV-4-ADDRESS>10.0.0.1</IPV-4-ADDRESS>
  </NETWORK-ENDPOINT>
  <APPLICATION-ENDPOINT><SHORT-NAME>Aep</SHORT-NAME>
   <NETWORK-ENDPOINT-REF>/P/Nep</NETWORK-ENDPOINT-REF>
   <TP-CONFIGURATION><UDP-TP><UDP-TP-PORT><PORT-NUMBER>30490</PORT-NUMBER>
   </UDP-TP-PORT></UDP-TP></TP-CONFIGURATION>
   <PROVIDED-SERVICE-INSTANCES>
    <PROVIDED-SERVICE-INSTANCE><SHORT-NAME>Display</SHORT-NAME>
     <SERVICE-IDENTIFIER>2001</SERVICE-IDENTIFIER>
     <INSTANCE-IDENTIFIER>1</INSTANCE-IDENTIFIER>
     <SD-SERVER-CONFIG>
      <SERVER-SERVICE-MAJOR-VERSION>1</SERVER-SERVICE-MAJOR-VERSION>
      <SERVER-SERVICE-MINOR-VERSION>0</SERVER-SERVICE-MINOR-VERSION>
     </SD-SERVER-CONFIG>
    </PROVIDED-SERVICE-INSTANCE>
   </PROVIDED-SERVICE-INSTANCES>
  </APPLICATION-ENDPOINT>
 </ELEMENTS></AR-PACKAGE></AR-PACKAGES>
</AUTOSAR>"#;
        let catalog = scan(text).expect("scan");
        assert_eq!(catalog.services.len(), 1);
        let service = &catalog.services[0];
        assert_eq!(service.id, 2001);
        assert_eq!(service.instances[0].instance, 1);
        assert_eq!(service.instances[0].endpoint, "10.0.0.1:30490");
    }
}
