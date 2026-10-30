//! ARXML reader that builds the unified model directly.
//!
//! One pass indexes every package-level object by path, then the cluster's
//! triggerings select the frames that belong to it. Signal and PDU definitions
//! are shared by the whole document, exactly as AUTOSAR states them.
//!
//! Advanced PDUs are read into the model (kinds, selectors, containers,
//! secured payload refs); the decode plan executes mux/container structure
//! and linear transformations, while E2E authentication stays unverified.

use roxmltree::{Document, Node};
use std::collections::{BTreeMap, HashMap, HashSet};

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScannedCluster {
    pub kind: ClusterKind,
    pub name: String,
}

/// Lists every CAN/FlexRay/Ethernet cluster in document order.
pub fn scan_clusters(text: &str) -> Result<Vec<ScannedCluster>, String> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let doc = Document::parse(text).map_err(|e| format!("XML parse error: {e}"))?;
    let mut found = Vec::new();
    for node in doc.descendants().filter(Node::is_element) {
        let kind = match node.tag_name().name() {
            "CAN-CLUSTER" => ClusterKind::Can,
            "FLEXRAY-CLUSTER" => ClusterKind::FlexRay,
            "ETHERNET-CLUSTER" => ClusterKind::Ethernet,
            _ => continue,
        };
        let name = short_name(node);
        if !name.is_empty() {
            found.push(ScannedCluster { kind, name });
        }
    }
    Ok(found)
}

pub struct ArxmlImporter;

impl ClusterImporter for ArxmlImporter {
    fn name(&self) -> &'static str {
        "arxml"
    }

    fn accepts(&self, source: &ClusterSource) -> bool {
        matches!(source, ClusterSource::Arxml { .. })
    }

    fn import(&self, input: ImportInput<'_>, ids: &mut ElementIds) -> ImportOutcome {
        let cluster = match input.source {
            ClusterSource::Arxml { cluster, .. } if !cluster.is_empty() => cluster.clone(),
            _ => {
                // An unnamed source means the first cluster in the file.
                let first = scan_clusters(input.text)
                    .ok()
                    .and_then(|clusters| clusters.into_iter().next());
                match first {
                    Some(cluster) => cluster.name,
                    None => {
                        return ImportOutcome {
                            comm: Comm::default(),
                            issues: vec![Issue::error(
                                IssueKind::Other,
                                "the ARXML file contains no communication cluster",
                            )],
                        };
                    }
                }
            }
        };
        import_cluster(input.text, &cluster, input.cluster, input.source, ids)
    }
}

pub fn import_cluster(
    text: &str,
    cluster_name: &str,
    cluster_id: ClusterId,
    source: &ClusterSource,
    ids: &mut ElementIds,
) -> ImportOutcome {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let doc = match Document::parse(text) {
        Ok(doc) => doc,
        Err(error) => {
            return ImportOutcome {
                comm: Comm::default(),
                issues: vec![Issue::error(
                    IssueKind::Other,
                    format!("XML parse error: {error}"),
                )],
            };
        }
    };
    let mut importer = Importer::new(&doc, cluster_id, source, ids);
    match importer.cluster_node(cluster_name) {
        Some(node) => importer.read_cluster(node),
        None => importer.issues.push(Issue::error(
            IssueKind::DanglingRef,
            format!("cluster '{cluster_name}' was not found in the file"),
        )),
    }
    let mut comm = importer.comm;
    comm.issues = importer.issues;
    ImportOutcome {
        issues: comm.issues.clone(),
        comm,
    }
}

struct Objects<'a> {
    by_path: BTreeMap<String, Node<'a, 'a>>,
    by_tail: BTreeMap<String, Vec<Node<'a, 'a>>>,
}

impl<'a> Objects<'a> {
    fn new(root: Node<'a, 'a>) -> Self {
        let mut by_path = BTreeMap::new();
        let mut by_tail: BTreeMap<String, Vec<Node<'a, 'a>>> = BTreeMap::new();
        for node in root.descendants().filter(Node::is_element) {
            if !indexed_tag(node.tag_name().name()) {
                continue;
            }
            let name = short_name(node);
            if name.is_empty() {
                continue;
            }
            by_path.entry(object_path(node)).or_insert(node);
            by_tail.entry(name).or_default().push(node);
        }
        Self { by_path, by_tail }
    }

    fn resolve(&self, reference: &str) -> Option<Node<'a, 'a>> {
        let reference = reference.trim();
        if let Some(node) = self.by_path.get(reference) {
            return Some(*node);
        }
        let tail = ref_tail(reference);
        self.by_tail
            .get(tail)
            .and_then(|nodes| (nodes.len() == 1).then(|| nodes[0]))
    }
}

struct Importer<'a> {
    root: Node<'a, 'a>,
    objects: Objects<'a>,
    cluster_id: ClusterId,
    network_id: NetworkId,
    source: ClusterSource,
    ids: &'a mut ElementIds,
    comm: Comm,
    issues: Vec<Issue>,
    units: HashMap<String, String>,
    compus: HashMap<String, CompuMethod>,
    system_compus: HashMap<String, CompuMethod>,
    system_base_types: HashMap<String, BaseType>,
    base_types: HashMap<String, BaseType>,
    system_signals: HashMap<String, SystemSignalId>,
    e2e_ids: HashMap<String, E2eProtectionId>,
    system_e2e: HashMap<SystemSignalId, E2eProtectionId>,
    signal_ids: HashMap<String, SignalId>,
    signal_pdus: HashMap<SignalId, PduId>,
    pdu_ids: HashMap<String, PduId>,
    frame_ids: HashMap<String, FrameId>,
}

impl<'a> Importer<'a> {
    fn new(
        doc: &'a Document<'a>,
        cluster_id: ClusterId,
        source: &ClusterSource,
        ids: &'a mut ElementIds,
    ) -> Self {
        let root = doc.root_element();
        Self {
            root,
            objects: Objects::new(root),
            cluster_id,
            network_id: NetworkId(cluster_id.0),
            source: source.clone(),
            ids,
            comm: Comm::default(),
            issues: Vec::new(),
            units: HashMap::new(),
            compus: HashMap::new(),
            system_compus: HashMap::new(),
            system_base_types: HashMap::new(),
            base_types: HashMap::new(),
            system_signals: HashMap::new(),
            e2e_ids: HashMap::new(),
            system_e2e: HashMap::new(),
            signal_ids: HashMap::new(),
            signal_pdus: HashMap::new(),
            pdu_ids: HashMap::new(),
            frame_ids: HashMap::new(),
        }
    }

    fn cluster_node(&self, name: &str) -> Option<Node<'a, 'a>> {
        ["CAN-CLUSTER", "FLEXRAY-CLUSTER", "ETHERNET-CLUSTER"]
            .iter()
            .flat_map(|tag| {
                self.root
                    .descendants()
                    .filter(move |node| node.has_tag_name(*tag))
            })
            .find(|node| short_name(*node).eq_ignore_ascii_case(name))
    }

    fn read_cluster(&mut self, cluster_node: Node<'a, 'a>) {
        let kind = match cluster_node.tag_name().name() {
            "CAN-CLUSTER" => ClusterKind::Can,
            "FLEXRAY-CLUSTER" => ClusterKind::FlexRay,
            _ => ClusterKind::Ethernet,
        };
        let name = short_name(cluster_node);
        self.comm.clusters.insert(
            self.cluster_id,
            Cluster {
                id: self.cluster_id,
                kind,
                name: name.clone(),
                aliases: Vec::new(),
                description: desc(cluster_node).unwrap_or_default(),
                sources: vec![self.source.clone()],
                networks: vec![self.network_id],
            },
        );
        let params = match kind {
            ClusterKind::Can => NetworkParams::Can(read_can_params(cluster_node)),
            ClusterKind::FlexRay => NetworkParams::FlexRay(read_flexray_params(cluster_node)),
            ClusterKind::Ethernet => NetworkParams::Ethernet(read_ethernet_params(cluster_node)),
        };
        self.comm.networks.insert(
            self.network_id,
            Network {
                id: self.network_id,
                cluster: self.cluster_id,
                name,
                params,
            },
        );

        self.read_catalog();
        self.read_system_signals();
        self.read_ecus();
        self.read_pdus();
        self.read_frames(cluster_node, kind);
        self.read_signal_groups();
        self.read_ports();
        let issues = self.comm.integrity_issues();
        self.issues.extend(issues);
    }

    /// Units, base types and compu methods are document-global.
    fn read_catalog(&mut self) {
        for node in self.root.descendants().filter(|n| n.has_tag_name("UNIT")) {
            let name = short_name(node);
            if name.is_empty() {
                continue;
            }
            let display = child_text(node, "DISPLAY-NAME").unwrap_or_else(|| name.clone());
            self.units.insert(name, display);
        }

        for node in self
            .root
            .descendants()
            .filter(|n| n.has_tag_name("SW-BASE-TYPE"))
        {
            let name = short_name(node);
            if name.is_empty() {
                continue;
            }
            let path = object_path(node);
            let id = BaseTypeId(self.ids.assign(
                ElementKind::BaseType,
                &source_key(self.cluster_id, "base", &path),
            ) as u32);
            let base = BaseType {
                id,
                name: name.clone(),
                size: child_number(node, "BASE-TYPE-SIZE").unwrap_or(0.0) as u32,
                encoding: child_text(node, "BASE-TYPE-ENCODING").unwrap_or_default(),
            };
            self.base_types.insert(name, base.clone());
            self.comm.base_types.insert(id, base);
        }

        for node in self
            .root
            .descendants()
            .filter(|n| n.has_tag_name("COMPU-METHOD"))
        {
            let name = short_name(node);
            if name.is_empty() {
                continue;
            }
            let path = object_path(node);
            let id = CompuMethodId(self.ids.assign(
                ElementKind::CompuMethod,
                &source_key(self.cluster_id, "compu", &path),
            ) as u32);
            let compu = read_compu(node, id, &self.units);
            self.compus.insert(name, compu.clone());
            self.comm.compu_methods.insert(id, compu);
        }

        // Vector matrices often state compu/base type on SYSTEM-SIGNAL and let
        // the I-SIGNAL refer to it.
        for node in self
            .root
            .descendants()
            .filter(|n| n.has_tag_name("SYSTEM-SIGNAL"))
        {
            let name = short_name(node);
            if name.is_empty() {
                continue;
            }
            if let Some(reference) = text_of(node, "COMPU-METHOD-REF")
                && let Some(compu) = self.compus.get(ref_tail(&reference))
            {
                self.system_compus.insert(name.clone(), compu.clone());
            }
            if let Some(reference) = text_of(node, "BASE-TYPE-REF")
                && let Some(base) = self.base_types.get(ref_tail(&reference))
            {
                self.system_base_types.insert(name, base.clone());
            }
        }

        for node in self
            .root
            .descendants()
            .filter(|n| n.has_tag_name("E2E-PROFILE-COMPATIBLE-PROTECTION"))
        {
            let name = short_name(node);
            if name.is_empty() {
                continue;
            }
            let path = object_path(node);
            let id = E2eProtectionId(
                self.ids
                    .assign(ElementKind::E2e, &source_key(self.cluster_id, "e2e", &path))
                    as u32,
            );
            let profile_name = child_text(node, "PROFILE-NAME").unwrap_or_default();
            let profile = profile_name
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u32>()
                .ok();
            let data_id = node
                .descendants()
                .find(|n| n.has_tag_name("DATA-ID"))
                .and_then(|n| n.text())
                .and_then(|text| text.trim().parse::<u32>().ok());
            let counter_offset = node
                .descendants()
                .find(|n| n.has_tag_name("COUNTER-OFFSET"))
                .and_then(|n| n.text())
                .and_then(|text| text.trim().parse::<u32>().ok());
            let counter_length = node
                .descendants()
                .find(|n| n.has_tag_name("COUNTER-LENGTH"))
                .and_then(|n| n.text())
                .and_then(|text| text.trim().parse::<u32>().ok());
            let protection = E2eProtection {
                id,
                name: name.clone(),
                profile_name,
                profile,
                data_id,
                system_signal: None,
                payload: None,
                counter_offset,
                counter_length,
            };
            self.e2e_ids.insert(name, id);
            self.e2e_ids.insert(path, id);
            self.comm.e2e.insert(id, protection);
        }
    }

    fn read_system_signals(&mut self) {
        for node in self
            .root
            .descendants()
            .filter(|n| n.has_tag_name("SYSTEM-SIGNAL"))
        {
            let name = short_name(node);
            if name.is_empty() {
                continue;
            }
            let path = object_path(node);
            let id = SystemSignalId(self.ids.assign(
                ElementKind::SystemSignal,
                &source_key(self.cluster_id, "system", &path),
            ) as u32);
            self.system_signals.insert(path.clone(), id);
            self.system_signals.insert(name.clone(), id);
            self.comm
                .system_signals
                .insert(id, SystemSignal { id, name });
            if let Some(reference) = text_of(node, "PROTECTION-PROPS-REF")
                && let Some(&e2e) = self.e2e_ids.get(ref_tail(&reference))
            {
                self.system_e2e.insert(id, e2e);
                if let Some(protection) = self.comm.e2e.get_mut(&e2e) {
                    protection.system_signal = Some(id);
                }
            }
        }
    }

    fn read_ecus(&mut self) {
        for node in self
            .root
            .descendants()
            .filter(|n| n.has_tag_name("ECU-INSTANCE"))
        {
            let name = short_name(node);
            if name.is_empty() {
                continue;
            }
            let path = object_path(node);
            let id = EcuId(
                self.ids
                    .assign(ElementKind::Ecu, &source_key(self.cluster_id, "ecu", &path))
                    as u32,
            );
            let mac = child_text(node, "MAC-UNICAST-ADDRESS").and_then(|text| parse_mac(&text));
            self.comm.ecus.insert(
                id,
                Ecu {
                    id,
                    name,
                    description: desc(node).unwrap_or_default(),
                    mac,
                },
            );
        }
    }

    fn read_pdus(&mut self) {
        let nodes: Vec<Node<'a, 'a>> = self
            .root
            .descendants()
            .filter(|n| {
                matches!(
                    n.tag_name().name(),
                    "I-SIGNAL-I-PDU" | "CONTAINER-I-PDU" | "MULTIPLEXED-I-PDU" | "SECURED-I-PDU"
                )
            })
            .collect();
        for node in &nodes {
            let path = object_path(*node);
            let name = short_name(*node);
            if name.is_empty() {
                continue;
            }
            let id = PduId(
                self.ids
                    .assign(ElementKind::Pdu, &source_key(self.cluster_id, "pdu", &path)),
            );
            self.pdu_ids.insert(path.clone(), id);
            self.pdu_ids.insert(name.clone(), id);
            let (kind, header_type, selector, secured) = read_pdu_shape(*node);
            let pdu = Pdu {
                id,
                name,
                length: child_number(*node, "LENGTH").unwrap_or(0.0) as u32,
                kind,
                header_type,
                cycle_us: pdu_cycle_us(*node),
                selector,
                secured,
                transformation: None,
                e2e: None,
            };
            self.comm.pdus.insert(id, pdu);
        }

        // SECURED-I-PDU/PAYLOAD-REF points at the protected PDU. Resolve it
        // after every PDU id exists so a forward reference works.
        for node in &nodes {
            if !node.has_tag_name("SECURED-I-PDU") {
                continue;
            }
            let Some(secured_id) = self.pdu_ids.get(&object_path(*node)).copied() else {
                continue;
            };
            let Some(payload) = ref_node(*node, "PAYLOAD-REF", &self.objects) else {
                continue;
            };
            // AUTOSAR 4.2 lets PAYLOAD-REF name a PDU-TRIGGERING, whose own
            // I-PDU-REF is the protected PDU (cantools' system-4.2 fixture is
            // written this way). Anything else is still expected to be a PDU.
            let mut payload_id = self.pdu_ids.get(&object_path(payload)).copied();
            if payload_id.is_none() {
                payload_id = ref_node(payload, "I-PDU-REF", &self.objects)
                    .or_else(|| ref_node(payload, "PDU-REF", &self.objects))
                    .and_then(|pdu| self.pdu_ids.get(&object_path(pdu)).copied());
            }
            let Some(payload_id) = payload_id else {
                let name = self
                    .comm
                    .pdus
                    .get(&secured_id)
                    .map(|pdu| pdu.name.clone())
                    .unwrap_or_else(|| "?".to_string());
                self.issues.push(Issue::warning(
                    IssueKind::Unsupported,
                    format!("secured PDU {name}: payload reference does not resolve to an I-PDU"),
                ));
                continue;
            };
            if let Some(pdu) = self.comm.pdus.get_mut(&secured_id)
                && let Some(meta) = &mut pdu.secured
            {
                meta.payload = Some(payload_id);
            }
        }

        // Signal mappings belong to the PDU that carries the I-Signal.
        for node in &nodes {
            if !node.has_tag_name("I-SIGNAL-I-PDU") {
                continue;
            }
            let Some(&pdu_id) = self.pdu_ids.get(&object_path(*node)) else {
                continue;
            };
            let mut seen: HashSet<SignalId> = HashSet::new();
            for (index, mapping) in node
                .descendants()
                .filter(|n| n.has_tag_name("I-SIGNAL-TO-I-PDU-MAPPING"))
                .enumerate()
            {
                let Some(signal_node) = ref_node(mapping, "I-SIGNAL-REF", &self.objects) else {
                    self.issues.push(
                        Issue::error(
                            IssueKind::DanglingRef,
                            format!(
                                "PDU {} has a signal mapping without a signal",
                                short_name(*node)
                            ),
                        )
                        .at(object_path(mapping)),
                    );
                    continue;
                };
                let signal_path = object_path(signal_node);
                let signal_id = match self.signal_ids.get(&signal_path) {
                    Some(id) => {
                        if self
                            .signal_pdus
                            .get(id)
                            .is_some_and(|existing| *existing != pdu_id)
                        {
                            self.issues.push(Issue::error(
                                IssueKind::SignalLayoutConflict,
                                format!(
                                    "signal {} is mapped into more than one PDU; only the first mapping is kept",
                                    short_name(signal_node)
                                ),
                            ));
                            continue;
                        }
                        if seen.insert(*id) {
                            *id
                        } else {
                            self.issues.push(Issue::warning(
                                IssueKind::DuplicateShortName,
                                format!(
                                    "signal {} is mapped more than once in PDU {}; the later mapping is ignored",
                                    short_name(signal_node),
                                    short_name(*node)
                                ),
                            ));
                            continue;
                        }
                    }
                    None => {
                        let id = SignalId(self.ids.assign(
                            ElementKind::Signal,
                            &source_key(self.cluster_id, "signal", &signal_path),
                        ));
                        let mut signal = self.read_signal(id, signal_node, pdu_id, mapping);
                        self.link_signal_transformation(&mut signal, signal_node);
                        self.comm.signals.insert(id, signal);
                        self.signal_ids.insert(signal_path.clone(), id);
                        self.signal_pdus.insert(id, pdu_id);
                        seen.insert(id);
                        id
                    }
                };
                let mapping_path = object_path(mapping);
                let mapping_id = MappingId(self.ids.assign(
                    ElementKind::Mapping,
                    &source_key(
                        self.cluster_id,
                        "signal-mapping",
                        &format!("{mapping_path}#{index}"),
                    ),
                ));
                let mapping_model = SignalPduMapping {
                    id: mapping_id,
                    signal: signal_id,
                    pdu: pdu_id,
                    start_bit: child_number(mapping, "START-POSITION").unwrap_or(0.0) as u32,
                    byte_order: byte_order(mapping),
                    update_bit: child_number(mapping, "UPDATE-INDICATION-BIT-POSITION")
                        .map(|v| v as u32),
                };
                if let Some(signal) = self.comm.signals.get_mut(&signal_id) {
                    signal.start_bit = mapping_model.start_bit;
                    signal.byte_order = mapping_model.byte_order;
                    signal.update_bit = mapping_model.update_bit;
                }
                self.comm
                    .signal_pdu_mappings
                    .insert(mapping_id, mapping_model);
            }
        }

        self.read_container_members();
        self.read_multiplexed_parts();
        let pdu_ids: Vec<PduId> = self.comm.pdus.keys().copied().collect();
        for pdu_id in pdu_ids {
            let protected = self
                .comm
                .signals
                .values()
                .filter(|signal| signal.pdu == pdu_id)
                .find_map(|signal| {
                    signal
                        .system_signal
                        .and_then(|system| self.system_e2e.get(&system).copied())
                });
            if let Some(e2e) = protected {
                if let Some(pdu) = self.comm.pdus.get_mut(&pdu_id) {
                    pdu.e2e = Some(e2e);
                }
                if let Some(protection) = self.comm.e2e.get_mut(&e2e) {
                    protection.payload = Some(pdu_id);
                }
            }
        }
    }

    /// Links an I-SIGNAL to its declared transformer chain. The chain's
    /// numeric behaviour is not interpreted here -- only the reference and
    /// its name are recorded, so the decode plan can report an unsupported
    /// transformation instead of silently treating the signal as raw.
    fn link_signal_transformation(&mut self, signal: &mut Signal, node: Node<'a, 'a>) {
        let Some(reference) = node
            .descendants()
            .filter(Node::is_element)
            .find(|n| n.has_tag_name("DATA-TRANSFORMATION-REF"))
            .and_then(|n| n.text())
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            return;
        };
        let name = ref_tail(reference).to_string();
        let id = TransformationId(self.ids.assign(
            ElementKind::Transformation,
            &source_key(self.cluster_id, "transformation", reference),
        ) as u32);
        let category = self
            .root
            .descendants()
            .filter(Node::is_element)
            .find(|n| {
                n.has_tag_name("DATA-TRANSFORMATION")
                    && (object_path(*n) == reference || ref_tail(&object_path(*n)) == name)
            })
            .and_then(|n| text_of(n, "PROTOCOL"))
            .unwrap_or_else(|| "custom".to_string());
        self.comm
            .transformations
            .entry(id)
            .or_insert(Transformation {
                id,
                name,
                kind: TransformationKind::Unknown { category },
            });
        signal.transformation = Some(id);
    }

    fn read_signal(
        &self,
        id: SignalId,
        node: Node<'a, 'a>,
        pdu: PduId,
        mapping: Node<'a, 'a>,
    ) -> Signal {
        let name = short_name(node);
        let system_ref = text_of(node, "SYSTEM-SIGNAL-REF");
        let system_tail = system_ref.as_deref().map(ref_tail);
        let base_ref = text_of(node, "BASE-TYPE-REF");
        let base = base_ref
            .as_deref()
            .and_then(|r| self.base_types.get(ref_tail(r)))
            .or_else(|| system_tail.and_then(|n| self.system_base_types.get(n)));
        let compu_ref = text_of(node, "COMPU-METHOD-REF");
        let compu = compu_ref
            .as_deref()
            .and_then(|r| self.compus.get(ref_tail(r)))
            .or_else(|| system_tail.and_then(|n| self.system_compus.get(n)))
            .cloned();
        let factor = compu.as_ref().map_or(1.0, |c| c.factor);
        let offset = compu.as_ref().map_or(0.0, |c| c.offset);
        let values = compu.as_ref().map(|c| c.values.clone()).unwrap_or_default();
        let unit = compu
            .as_ref()
            .and_then(|c| c.unit.clone())
            .unwrap_or_default();
        let (mut min, mut max, mut min_open, mut max_open) = compu
            .as_ref()
            .map(|c| (c.min, c.max, c.min_open, c.max_open))
            .unwrap_or((None, None, false, false));
        if min.is_none() || max.is_none() {
            let data_constr = system_ref
                .as_deref()
                .and_then(|reference| self.objects.resolve(reference))
                .or(Some(node));
            if let Some((lower, upper, lower_open, upper_open)) =
                data_constr.and_then(|definition| self.data_constr_limits(definition))
            {
                min = min.or(lower);
                max = max.or(upper);
                min_open |= lower_open;
                max_open |= upper_open;
            }
        }
        let system_signal = system_ref.as_deref().and_then(|r| self.system_signal_id(r));
        let length = child_number(node, "LENGTH").unwrap_or(0.0).max(0.0) as u32;
        let signed = base.is_some_and(BaseType::signed);
        let float = base.is_some_and(BaseType::float);
        let initial_raw = node
            .descendants()
            .find(|n| n.has_tag_name("INIT-VALUE"))
            .and_then(|init| first_number(init))
            .or_else(|| {
                system_ref
                    .as_deref()
                    .and_then(|reference| self.objects.resolve(reference))
                    .and_then(|system| {
                        system
                            .descendants()
                            .find(|n| n.has_tag_name("INIT-VALUE"))
                            .and_then(|init| first_number(init))
                    })
            });
        Signal {
            id,
            pdu,
            name,
            system_signal,
            length,
            start_bit: child_number(mapping, "START-POSITION").unwrap_or(0.0) as u32,
            byte_order: byte_order(mapping),
            update_bit: child_number(mapping, "UPDATE-INDICATION-BIT-POSITION").map(|v| v as u32),
            signed,
            float,
            factor,
            offset,
            unit,
            min,
            max,
            min_open,
            max_open,
            initial_raw,
            values,
            base_type: base.map(|b| b.id),
            compu: compu.map(|c| c.id),
            comment: desc(node).unwrap_or_default(),
            transformation: None,
            group: None,
            mux: Vec::new(),
        }
    }

    fn read_frames(&mut self, cluster_node: Node<'a, 'a>, kind: ClusterKind) {
        let trigger_tag = match kind {
            ClusterKind::Can => "CAN-FRAME-TRIGGERING",
            ClusterKind::FlexRay => "FLEXRAY-FRAME-TRIGGERING",
            ClusterKind::Ethernet => "ETHERNET-FRAME-TRIGGERING",
        };
        let mut triggered: HashSet<String> = HashSet::new();

        for triggering in cluster_node
            .descendants()
            .filter(|n| n.has_tag_name(trigger_tag))
        {
            let Some(frame_node) = ref_node(triggering, "FRAME-REF", &self.objects) else {
                self.issues.push(
                    Issue::error(
                        IssueKind::DanglingRef,
                        format!("{trigger_tag} names no frame"),
                    )
                    .at(object_path(triggering)),
                );
                continue;
            };
            let frame_path = object_path(frame_node);
            let name = short_name(frame_node);
            if !triggered.insert(frame_path.clone()) {
                if let (FrameAddress::Ethernet { vlan: first }, FrameAddress::Ethernet { vlan }) = (
                    self.frame_ids
                        .get(&frame_path)
                        .and_then(|id| self.comm.frames.get(id))
                        .map_or(FrameAddress::Ethernet { vlan: None }, |f| f.address),
                    self.read_address(triggering, frame_node, kind),
                ) && first != vlan
                {
                    self.issues.push(Issue::warning(
                        IssueKind::AmbiguousFrame,
                        format!(
                            "frame {name} is triggered on more than one VLAN; the first one is kept"
                        ),
                    ));
                }
                continue;
            }
            let frame_id = FrameId(self.ids.assign(
                ElementKind::Frame,
                &source_key(self.cluster_id, "frame", &frame_path),
            ));
            self.frame_ids.insert(frame_path.clone(), frame_id);
            self.frame_ids.insert(name.clone(), frame_id);
            let address = self.read_address(triggering, frame_node, kind);
            let mut frame = Frame {
                id: frame_id,
                cluster: self.cluster_id,
                network: self.network_id,
                name: name.clone(),
                length: child_number(frame_node, "FRAME-LENGTH").unwrap_or(0.0) as u32,
                address,
                timing: read_timing(triggering),
                comment: desc(frame_node).unwrap_or_default(),
                ethernet: read_ethernet_meta(frame_node),
                pdu_mappings: Vec::new(),
            };
            self.read_frame_pdus(frame_node, &mut frame);
            if frame.timing.cycle_us.is_none() {
                frame.timing.cycle_us = frame.pdu_mappings.iter().find_map(|mapping| {
                    self.comm
                        .frame_pdu_mappings
                        .get(mapping)
                        .and_then(|mapping| self.comm.pdus.get(&mapping.pdu))
                        .and_then(|pdu| pdu.cycle_us)
                });
            }
            self.comm.frames.insert(frame_id, frame);
        }
    }

    fn read_address(
        &self,
        triggering: Node<'a, 'a>,
        frame: Node<'a, 'a>,
        kind: ClusterKind,
    ) -> FrameAddress {
        match kind {
            ClusterKind::Can => {
                let id = child_number(triggering, "IDENTIFIER")
                    .or_else(|| child_number(triggering, "CAN-ID"))
                    .unwrap_or(0.0) as u32;
                let mode = child_text(triggering, "CAN-ADDRESSING-MODE").unwrap_or_default();
                let extended = mode.eq_ignore_ascii_case("EXTENDED") || id > 0x7FF;
                let fd = child_text(triggering, "CAN-FD-FRAME-SUPPORT")
                    .is_some_and(|v| v.eq_ignore_ascii_case("true"))
                    || (child_number(frame, "FRAME-LENGTH").unwrap_or(0.0) > 8.0 && self.can_fd());
                FrameAddress::Can { id, extended, fd }
            }
            ClusterKind::FlexRay => FrameAddress::FlexRay {
                slot: any_number(triggering, "SLOT-ID").unwrap_or(0.0) as u16,
                base_cycle: any_number(triggering, "BASE-CYCLE").unwrap_or(0.0) as u8,
                repetition: repetition(triggering),
                payload_preamble: child_text(triggering, "PAYLOAD-PREAMBLE-INDICATOR")
                    .is_some_and(|v| v.eq_ignore_ascii_case("true")),
            },
            ClusterKind::Ethernet => FrameAddress::Ethernet {
                vlan: triggering_vlan(triggering),
            },
        }
    }

    fn can_fd(&self) -> bool {
        matches!(
            self.comm.networks.get(&self.network_id).map(|n| &n.params),
            Some(NetworkParams::Can(p)) if p.supports_fd()
        )
    }

    fn read_frame_pdus(&mut self, frame_node: Node<'a, 'a>, frame: &mut Frame) {
        let frame_path = object_path(frame_node);
        for (index, mapping) in frame_node
            .descendants()
            .filter(|n| n.has_tag_name("PDU-TO-FRAME-MAPPING"))
            .enumerate()
        {
            let Some(pdu_node) = ref_node(mapping, "PDU-REF", &self.objects) else {
                self.issues.push(
                    Issue::error(
                        IssueKind::DanglingRef,
                        format!("frame {} has a PDU mapping without a PDU", frame.name),
                    )
                    .at(object_path(mapping)),
                );
                continue;
            };
            let Some(&pdu_id) = self.pdu_ids.get(&object_path(pdu_node)) else {
                self.issues.push(
                    Issue::error(
                        IssueKind::DanglingRef,
                        format!(
                            "frame {} refers to PDU {}, which was not read",
                            frame.name,
                            short_name(pdu_node)
                        ),
                    )
                    .at(object_path(mapping)),
                );
                continue;
            };
            let mapping_id = MappingId(self.ids.assign(
                ElementKind::Mapping,
                &source_key(
                    self.cluster_id,
                    "frame-mapping",
                    &format!("{frame_path}:{index}:{}", object_path(mapping)),
                ),
            ));
            self.comm.frame_pdu_mappings.insert(
                mapping_id,
                FramePduMapping {
                    id: mapping_id,
                    frame: frame.id,
                    pdu: pdu_id,
                    start_bit: child_number(mapping, "START-POSITION").unwrap_or(0.0) as u32,
                    byte_order: byte_order(mapping),
                },
            );
            frame.pdu_mappings.push(mapping_id);
        }
    }

    fn read_container_members(&mut self) {
        let containers: Vec<Node<'a, 'a>> = self
            .root
            .descendants()
            .filter(|n| n.has_tag_name("CONTAINER-I-PDU"))
            .collect();
        for container_node in containers {
            let Some(&container_id) = self.pdu_ids.get(&object_path(container_node)) else {
                continue;
            };
            for (index, reference) in reference_list(container_node, "CONTAINED-PDU-TRIGGERING-REF")
                .into_iter()
                .enumerate()
            {
                let Some(triggering) = self.objects.resolve(&reference) else {
                    self.issues.push(Issue::error(
                        IssueKind::DanglingRef,
                        format!(
                            "container {} refers to a missing PDU triggering {reference}",
                            short_name(container_node)
                        ),
                    ));
                    continue;
                };
                let Some(child) = ref_node(triggering, "I-PDU-REF", &self.objects) else {
                    continue;
                };
                let Some(&child_id) = self.pdu_ids.get(&object_path(child)) else {
                    continue;
                };
                let mapping_id = MappingId(self.ids.assign(
                    ElementKind::Mapping,
                    &source_key(
                        self.cluster_id,
                        "container-mapping",
                        &format!("{}:{index}", object_path(container_node)),
                    ),
                ));
                self.comm.contained_pdu_mappings.insert(
                    mapping_id,
                    ContainedPduMapping {
                        id: mapping_id,
                        container: container_id,
                        pdu: child_id,
                        header_id: contained_header_id(child, triggering),
                        offset: contained_offset(child),
                    },
                );
            }
        }
    }

    fn read_multiplexed_parts(&mut self) {
        let muxes: Vec<Node<'a, 'a>> = self
            .root
            .descendants()
            .filter(|n| n.has_tag_name("MULTIPLEXED-I-PDU"))
            .collect();
        for mux_node in muxes {
            let Some(&mux_id) = self.pdu_ids.get(&object_path(mux_node)) else {
                continue;
            };
            let mut index = 0;
            for alternative in mux_node
                .descendants()
                .filter(|n| n.has_tag_name("DYNAMIC-PART-ALTERNATIVE"))
            {
                let Some(child) = ref_node(alternative, "I-PDU-REF", &self.objects) else {
                    continue;
                };
                let Some(&child_id) = self.pdu_ids.get(&object_path(child)) else {
                    continue;
                };
                let mapping_id = MappingId(self.ids.assign(
                    ElementKind::Mapping,
                    &source_key(
                        self.cluster_id,
                        "mux-mapping",
                        &format!("{}:dynamic:{index}", object_path(mux_node)),
                    ),
                ));
                self.comm.multiplexed_parts.insert(
                    mapping_id,
                    MultiplexedPduPart {
                        id: mapping_id,
                        mux: mux_id,
                        part: child_id,
                        selector_code: child_number(alternative, "SELECTOR-FIELD-CODE")
                            .map(|v| v as u32),
                        initial: child_text(alternative, "INITIAL-DYNAMIC-PART")
                            .is_some_and(|v| v.eq_ignore_ascii_case("true")),
                    },
                );
                index += 1;
            }
            for static_part in mux_node
                .descendants()
                .filter(|n| n.has_tag_name("STATIC-PART"))
            {
                let Some(child) = ref_node(static_part, "I-PDU-REF", &self.objects) else {
                    continue;
                };
                let Some(&child_id) = self.pdu_ids.get(&object_path(child)) else {
                    continue;
                };
                let mapping_id = MappingId(self.ids.assign(
                    ElementKind::Mapping,
                    &source_key(
                        self.cluster_id,
                        "mux-mapping",
                        &format!("{}:static:{index}", object_path(mux_node)),
                    ),
                ));
                self.comm.multiplexed_parts.insert(
                    mapping_id,
                    MultiplexedPduPart {
                        id: mapping_id,
                        mux: mux_id,
                        part: child_id,
                        selector_code: None,
                        initial: false,
                    },
                );
                index += 1;
            }
        }
    }

    /// E2E declared on a signal group through a transformation set. The
    /// referenced TRANSFORMATION-TECHNOLOGY carries the profile name; the group
    /// carries the data ids. Neither states where the counter or CRC sit, so this
    /// is recorded for the UI and reported as unsupported for planning.
    fn read_group_e2e(node: Node<'_, '_>, objects: &Objects<'_>) -> Option<GroupE2eProtection> {
        let reference = node
            .descendants()
            .find(|n| n.has_tag_name("TRANSFORMER-REF"))?
            .text()?
            .trim()
            .to_string();
        let technology = objects.resolve(&reference)?;
        let profile_name = technology
            .descendants()
            .find(|n| n.has_tag_name("PROFILE-NAME"))
            .and_then(|n| n.text())
            .unwrap_or_default()
            .to_string();
        if profile_name.is_empty() {
            return None;
        }
        let profile = profile_name
            .trim()
            .strip_prefix("Profile")
            .and_then(|number| number.trim().parse::<u32>().ok());
        let data_ids = node
            .descendants()
            .filter(|n| n.has_tag_name("DATA-ID"))
            .filter_map(|n| n.text())
            .filter_map(|text| text.trim().parse::<u32>().ok())
            .collect();
        Some(GroupE2eProtection {
            profile_name,
            profile,
            data_ids,
        })
    }

    fn read_signal_groups(&mut self) {
        let groups: Vec<Node<'a, 'a>> = self
            .root
            .descendants()
            .filter(|n| n.has_tag_name("I-SIGNAL-GROUP"))
            .collect();
        for group_node in groups {
            let name = short_name(group_node);
            if name.is_empty() {
                continue;
            }
            let path = object_path(group_node);
            let id = SignalGroupId(self.ids.assign(
                ElementKind::SignalGroup,
                &source_key(self.cluster_id, "group", &path),
            ) as u32);
            let e2e = Self::read_group_e2e(group_node, &self.objects);
            self.comm.groups.insert(
                id,
                SignalGroup {
                    id,
                    name: name.clone(),
                    sender: None,
                    e2e: e2e.clone(),
                },
            );
            if let Some(e2e) = &e2e {
                self.issues.push(Issue::warning(
                    IssueKind::Unsupported,
                    format!(
                        "signal group {name}: E2E {} is declared through a transformation set \
                         without counter/CRC positions; frame-level counter continuity is not planned",
                        e2e.profile_name
                    ),
                ));
            }
            for reference in reference_list(group_node, "I-SIGNAL-REF") {
                if let Some(signal_node) = self.objects.resolve(&reference)
                    && let Some(&signal_id) = self.signal_ids.get(&object_path(signal_node))
                    && let Some(signal) = self.comm.signals.get_mut(&signal_id)
                {
                    signal.group = Some(id);
                }
            }
        }
    }

    fn read_ports(&mut self) {
        for port in self
            .root
            .descendants()
            .filter(|n| n.has_tag_name("FRAME-PORT"))
        {
            let Some(ecu_id) = self.ecu_of(port) else {
                continue;
            };
            let direction = child_text(port, "COMMUNICATION-DIRECTION").unwrap_or_default();
            let role = if direction.eq_ignore_ascii_case("OUT") {
                RelationRole::Sender
            } else {
                RelationRole::Receiver
            };
            let Some(frame) = ref_node(port, "FRAME-REF", &self.objects) else {
                continue;
            };
            let Some(&frame_id) = self.frame_ids.get(&object_path(frame)) else {
                continue;
            };
            self.comm.frame_relations.push(FrameEcuRelation {
                frame: frame_id,
                ecu: ecu_id,
                role,
            });
        }
        for port in self
            .root
            .descendants()
            .filter(|n| n.has_tag_name("PDU-PORT"))
        {
            let Some(ecu_id) = self.ecu_of(port) else {
                continue;
            };
            let direction = child_text(port, "COMMUNICATION-DIRECTION").unwrap_or_default();
            let role = if direction.eq_ignore_ascii_case("OUT") {
                RelationRole::Sender
            } else {
                RelationRole::Receiver
            };
            let pdu = ref_node(port, "I-PDU-REF", &self.objects)
                .or_else(|| ref_node(port, "PDU-REF", &self.objects));
            let Some(pdu) = pdu else {
                continue;
            };
            let Some(&pdu_id) = self.pdu_ids.get(&object_path(pdu)) else {
                continue;
            };
            self.comm.pdu_relations.push(PduEcuRelation {
                pdu: pdu_id,
                ecu: ecu_id,
                role,
            });
        }
    }

    fn ecu_of(&self, node: Node<'a, 'a>) -> Option<EcuId> {
        node.ancestors()
            .find(|n| n.has_tag_name("ECU-INSTANCE"))
            .and_then(|ecu| {
                let path = object_path(ecu);
                self.ids
                    .id_for(ElementKind::Ecu, &source_key(self.cluster_id, "ecu", &path))
                    .map(|id| EcuId(id as u32))
            })
            .filter(|id| self.comm.ecus.contains_key(id))
    }

    fn system_signal_id(&self, reference: &str) -> Option<SystemSignalId> {
        self.system_signals
            .get(reference)
            .or_else(|| self.system_signals.get(ref_tail(reference)))
            .copied()
    }

    fn data_constr_limits(
        &self,
        definition: Node<'a, 'a>,
    ) -> Option<(Option<f64>, Option<f64>, bool, bool)> {
        let reference = text_of(definition, "DATA-CONSTR-REF")?;
        let target = self.objects.resolve(&reference)?;
        let constrs = target
            .descendants()
            .find(|n| n.has_tag_name("PHYS-CONSTRS"))
            .unwrap_or(target);
        let lower = constrs
            .descendants()
            .find(|n| n.has_tag_name("LOWER-LIMIT"));
        let upper = constrs
            .descendants()
            .find(|n| n.has_tag_name("UPPER-LIMIT"));
        if lower.is_none() && upper.is_none() {
            return None;
        }
        let number = |node: Option<Node<'a, 'a>>| {
            node.and_then(|n| n.text())
                .and_then(|text| text.trim().parse::<f64>().ok())
        };
        let open = |node: Option<Node<'a, 'a>>| {
            node.and_then(|n| n.attribute("INTERVAL-TYPE"))
                .is_some_and(|kind| kind.eq_ignore_ascii_case("OPEN"))
        };
        Some((number(lower), number(upper), open(lower), open(upper)))
    }
}

fn source_key(cluster: ClusterId, kind: &str, path: &str) -> String {
    format!("arxml:{}:{kind}:{path}", cluster.0)
}

fn read_can_params(cluster: Node<'_, '_>) -> CanParams {
    let bitrate = any_number(cluster, "BAUDRATE").or_else(|| any_number(cluster, "SPEED"));
    let fd =
        any_number(cluster, "CAN-FD-BAUDRATE").or_else(|| any_number(cluster, "CAN-FD-DATA-RATE"));
    CanParams {
        bitrate_kbps: (bitrate.unwrap_or(0.0) / 1000.0).round() as u32,
        fd_data_kbps: (fd.unwrap_or(0.0) / 1000.0).round() as u32,
        sample_point_permille: None,
    }
}

fn read_flexray_params(cluster: Node<'_, '_>) -> FlexRayParams {
    let speed = any_number(cluster, "BAUDRATE")
        .or_else(|| any_number(cluster, "SPEED"))
        .or_else(|| any_number(cluster, "BIT-RATE"))
        .unwrap_or(0.0);
    let cycle = child_named_number(
        cluster,
        &["CYCLE-TIME-ms", "CYCLE-TIME-MS", "CYCLE-TIME", "gdCycle"],
    )
    .unwrap_or(0.0);
    let mut extra = BTreeMap::new();
    for tag in [
        "NUMBER-OF-MINISLOTS",
        "G-NUMBER-OF-MINISLOTS",
        "MINISLOT-DURATION",
        "STATIC-SLOT-LENGTH",
        "PAYLOAD-LENGTH-STATIC",
    ] {
        if let Some(value) = child_text(cluster, tag) {
            extra.insert(tag.to_string(), value);
        }
    }
    FlexRayParams {
        speed_kbps: speed as u32,
        cycle_time_ms: cycle,
        macrotick_us: child_named_number(
            cluster,
            &["MACROTICK", "MACROTICK-DURATION", "GD-MACROTICK-DURATION"],
        )
        .unwrap_or(0.0),
        coldstart_attempts: child_named_number(
            cluster,
            &["COLDSTART-ATTEMPTS", "GD-COLDSTART-ATTEMPTS"],
        )
        .unwrap_or(0.0) as u32,
        action_point_offset: child_named_number(
            cluster,
            &["ACTION-POINT-OFFSET", "GD-ACTION-POINT-OFFSET"],
        )
        .unwrap_or(0.0) as u32,
        minislot_action_point_offset: child_named_number(
            cluster,
            &[
                "MINISLOT-ACTION-POINT-OFFSET",
                "GD-MINISLOT-ACTION-POINT-OFFSET",
            ],
        )
        .unwrap_or(0.0) as u32,
        dynamic_slot_idle_phase: child_named_number(
            cluster,
            &["DYNAMIC-SLOT-IDLE-PHASE", "GD-DYNAMIC-SLOT-IDLE-PHASE"],
        )
        .unwrap_or(0.0) as u32,
        minor_version: child_named_number(cluster, &["MINOR-VERSION", "GD-MINOR-VERSION"])
            .unwrap_or(0.0) as u32,
        extra,
    }
}

fn read_ethernet_params(cluster: Node<'_, '_>) -> EthernetParams {
    let layer = cluster
        .descendants()
        .find(|n| n.has_tag_name("PHYSICAL-LAYER-TYPE"))
        .and_then(|n| n.text())
        .and_then(EthernetPhy::from_xml);
    let mut vlans = Vec::new();
    for channel in cluster
        .descendants()
        .filter(|n| n.has_tag_name("ETHERNET-PHYSICAL-CHANNEL"))
    {
        if let Some(vlan) = triggering_vlan(channel)
            && !vlans.contains(&vlan)
        {
            vlans.push(vlan);
        }
    }
    EthernetParams {
        physical_layer: layer,
        vlans,
        frame_length_includes_header: false,
    }
}

fn repetition(node: Node<'_, '_>) -> u8 {
    if let Some(value) = any_number(node, "CYCLE-REPETITION") {
        return value.max(1.0) as u8;
    }
    node.descendants()
        .filter(|n| n.has_tag_name("CYCLE-REPETITION"))
        .find_map(|n| n.text())
        .and_then(|text| {
            let suffix = text
                .trim()
                .rsplit('-')
                .next()
                .and_then(|part| part.parse::<u8>().ok())?;
            // AUTOSAR spells the repetition as cycle-repetition-2, -4, -8...
            // while the model stores the repetition exponent's value (2, 4).
            Some(suffix.max(1))
        })
        .unwrap_or(1)
}

fn pdu_cycle_us(node: Node<'_, '_>) -> Option<u64> {
    if let Some(period) = node
        .descendants()
        .find(|n| n.has_tag_name("TIME-PERIOD"))
        .and_then(|period| first_number(period))
        .filter(|seconds| *seconds > 0.0)
    {
        return Some((period * 1_000_000.0).round() as u64);
    }
    any_number(node, "CYCLE-TIME")
        .filter(|ms| *ms > 0.0)
        .map(|ms| (ms * 1000.0).round() as u64)
}

fn read_timing(node: Node<'_, '_>) -> FrameTiming {
    let cycle_us = child_named_number(
        node,
        &[
            "CYCLE-TIME",
            "CYCLE-TIME-ms",
            "CYCLE-TIME-MS",
            "TIME-PERIOD",
        ],
    )
    .map(|ms| (ms * 1000.0).round() as u64);
    FrameTiming {
        cycle_us,
        startup: child_text(node, "STARTUP-FRAME").is_some_and(|v| v.eq_ignore_ascii_case("true")),
        event: child_text(node, "TRIGGERING-MODE").is_some_and(|v| v.eq_ignore_ascii_case("EVENT")),
    }
}

fn read_ethernet_meta(frame: Node<'_, '_>) -> Option<EthernetFrameMeta> {
    match frame.tag_name().name() {
        "GENERIC-ETHERNET-FRAME" => Some(EthernetFrameMeta::Generic),
        "USER-DEFINED-ETHERNET-FRAME" => Some(EthernetFrameMeta::UserDefined),
        "IEEE-1722-TP-ETHERNET-FRAME" => Some(EthernetFrameMeta::Ieee1722 {
            stream_identifier: child_number(frame, "STREAM-IDENTIFIER").unwrap_or(0.0) as u64,
            sub_type: child_number(frame, "SUB-TYPE").unwrap_or(0.0) as u64,
            version: child_number(frame, "VERSION").unwrap_or(0.0) as u64,
            presentation_time_s: child_number(frame, "RELATIVE-REPRESENTATION-TIME"),
        }),
        _ => Some(EthernetFrameMeta::Base),
    }
}

/// The authenticator length stated on the authentication-props object a
/// secured PDU references. System descriptions usually carry the
/// authenticator size there rather than inline on the PDU.
fn referenced_auth_info_length(node: Node<'_, '_>) -> Option<f64> {
    let root = node.ancestors().last()?;
    let reference = node
        .descendants()
        .find(|n| n.has_tag_name("AUTHENTICATION-PROPS-REF"))?
        .text()?
        .trim()
        .to_string();
    let tail = reference.rsplit('/').next()?.trim();
    root.descendants()
        .find(|n| {
            n.has_tag_name("SECURE-COMMUNICATION-AUTHENTICATION-PROPS") && short_name(*n) == tail
        })
        .and_then(|n| child_number(n, "AUTH-INFO-TX-LENGTH"))
}

fn read_pdu_shape(
    node: Node<'_, '_>,
) -> (
    PduKind,
    ContainerHeaderType,
    Option<SelectorField>,
    Option<SecuredPduMeta>,
) {
    match node.tag_name().name() {
        "CONTAINER-I-PDU" => {
            let header_type = match child_text(node, "HEADER-TYPE").as_deref() {
                Some("LONG-HEADER") => ContainerHeaderType::Long,
                Some("NO-HEADER") => ContainerHeaderType::None,
                _ => ContainerHeaderType::Short,
            };
            (PduKind::Container, header_type, None, None)
        }
        "MULTIPLEXED-I-PDU" => {
            let selector = SelectorField {
                start_bit: child_number(node, "SELECTOR-FIELD-START-POSITION").unwrap_or(0.0)
                    as u32,
                length: child_number(node, "SELECTOR-FIELD-LENGTH").unwrap_or(0.0) as u32,
                byte_order: byte_order(node),
                signal: None,
            };
            (
                PduKind::Multiplexed,
                ContainerHeaderType::Short,
                Some(selector),
                None,
            )
        }
        "SECURED-I-PDU" => {
            let header = match child_text(node, "USE-SECURED-PDU-HEADER").as_deref() {
                Some("SECURED-PDU-HEADER-08-BIT") => SecuredPduHeader::Header08,
                Some("SECURED-PDU-HEADER-16-BIT") => SecuredPduHeader::Header16,
                Some("SECURED-PDU-HEADER-32-BIT") => SecuredPduHeader::Header32,
                _ => SecuredPduHeader::NoHeader,
            };
            let props = node
                .descendants()
                .find(|n| n.has_tag_name("SECURE-COMMUNICATION-PROPS"));
            let number = |tag: &str| props.and_then(|p| child_number(p, tag)).map(|v| v as u64);
            // The secured area and the authenticator length live inside
            // SECURE-COMMUNICATION-PROPS in system descriptions; some tools
            // also put the authenticator length on the referenced
            // authentication-props object instead.
            let secured_area = props.and_then(|p| {
                child_number(p, "SECURED-AREA-OFFSET").and_then(|offset| {
                    child_number(p, "SECURED-AREA-LENGTH")
                        .map(|length| (offset as u32, length as u32))
                })
            });
            let auth_info_length = props
                .and_then(|p| {
                    child_number(p, "AUTH-INFO-LENGTH")
                        .or_else(|| child_number(p, "AUTH-INFO-TX-LENGTH"))
                })
                .or_else(|| referenced_auth_info_length(node))
                .map(|v| v as u32);
            let secured = SecuredPduMeta {
                header,
                payload: None,
                data_id: number("DATA-ID"),
                freshness_value_id: number("FRESHNESS-VALUE-ID"),
                secured_area,
                auth_info_length,
                verification: SecVerification::Unverified,
            };
            (
                PduKind::Secured,
                ContainerHeaderType::Short,
                None,
                Some(secured),
            )
        }
        _ => (PduKind::Signal, ContainerHeaderType::Short, None, None),
    }
}

fn read_compu(
    node: Node<'_, '_>,
    id: CompuMethodId,
    units: &HashMap<String, String>,
) -> CompuMethod {
    let mut factor = 1.0;
    let mut offset = 0.0;
    let mut values = Vec::new();
    let mut min = None;
    let mut max = None;
    let mut min_open = false;
    let mut max_open = false;
    for scale in node.descendants().filter(|n| n.has_tag_name("COMPU-SCALE")) {
        if let Some(limit) = scale.descendants().find(|n| n.has_tag_name("LOWER-LIMIT")) {
            min = limit.text().and_then(|t| t.trim().parse().ok());
            min_open = limit
                .attribute("INTERVAL-TYPE")
                .is_some_and(|v| v.eq_ignore_ascii_case("OPEN"));
        }
        if let Some(limit) = scale.descendants().find(|n| n.has_tag_name("UPPER-LIMIT")) {
            max = limit.text().and_then(|t| t.trim().parse().ok());
            max_open = limit
                .attribute("INTERVAL-TYPE")
                .is_some_and(|v| v.eq_ignore_ascii_case("OPEN"));
        }
        if let Some(coeffs) = scale
            .descendants()
            .find(|n| n.has_tag_name("COMPU-RATIONAL-COEFFS"))
        {
            let numerator = coeffs
                .descendants()
                .find(|n| n.has_tag_name("COMPU-NUMERATOR"))
                .map(|n| numbers(n, "V"))
                .unwrap_or_default();
            let denominator = coeffs
                .descendants()
                .find(|n| n.has_tag_name("COMPU-DENOMINATOR"))
                .map(|n| numbers(n, "V"))
                .unwrap_or_else(|| vec![1.0]);
            let den = denominator
                .first()
                .copied()
                .filter(|v| *v != 0.0)
                .unwrap_or(1.0);
            factor = numerator.get(1).copied().unwrap_or(1.0) / den;
            offset = numerator.first().copied().unwrap_or(0.0) / den;
        }
        if let Some(constant) = scale.descendants().find(|n| n.has_tag_name("COMPU-CONST")) {
            let code = child_number(constant, "V").map(|v| v as i64);
            let text = child_text(constant, "VT");
            if let (Some(code), Some(text)) = (code, text) {
                values.push((code, text));
            }
        }
    }
    let unit = text_of(node, "UNIT-REF")
        .as_deref()
        .map(ref_tail)
        .and_then(|name| units.get(name).cloned());
    let category = if !values.is_empty() {
        CompuCategory::TextTable
    } else if factor == 1.0 && offset == 0.0 {
        CompuCategory::Identical
    } else {
        CompuCategory::Linear
    };
    CompuMethod {
        id,
        name: short_name(node),
        category,
        factor,
        offset,
        values,
        unit,
        min,
        max,
        min_open,
        max_open,
    }
}

fn contained_header_id(child: Node<'_, '_>, triggering: Node<'_, '_>) -> u32 {
    for tag in ["HEADER-ID-SHORT-HEADER", "HEADER-ID-LONG-HEADER"] {
        if let Some(value) = any_number(child, tag) {
            return value as u32;
        }
    }
    any_number(triggering, "HEADER-ID").unwrap_or(0.0) as u32
}

fn contained_offset(child: Node<'_, '_>) -> u32 {
    child
        .descendants()
        .find(|n| n.has_tag_name("OFFSET"))
        .and_then(|n| n.text())
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0.0) as u32
}

fn triggering_vlan(node: Node<'_, '_>) -> Option<u16> {
    let channel = node
        .ancestors()
        .find(|n| n.has_tag_name("ETHERNET-PHYSICAL-CHANNEL"))
        .or_else(|| node.is_element().then_some(node))?;
    for tag in ["VLAN-CONFIG", "VLAN"] {
        if let Some(config) = channel.children().find(|n| n.has_tag_name(tag))
            && let Some(vlan) = child_number(config, "VLAN-IDENTIFIER")
        {
            return Some(vlan as u16);
        }
    }
    None
}

fn ref_node<'a>(node: Node<'a, 'a>, tag: &str, objects: &Objects<'a>) -> Option<Node<'a, 'a>> {
    text_of(node, tag).and_then(|reference| objects.resolve(&reference))
}

fn reference_list<'a>(node: Node<'a, 'a>, tag: &str) -> Vec<String> {
    node.descendants()
        .filter(|n| n.has_tag_name(tag))
        .filter_map(|n| n.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn indexed_tag(tag: &str) -> bool {
    matches!(
        tag,
        "CAN-CLUSTER"
            | "FLEXRAY-CLUSTER"
            | "ETHERNET-CLUSTER"
            | "CAN-FRAME"
            | "FLEXRAY-FRAME"
            | "ETHERNET-FRAME"
            | "GENERIC-ETHERNET-FRAME"
            | "USER-DEFINED-ETHERNET-FRAME"
            | "IEEE-1722-TP-ETHERNET-FRAME"
            | "I-SIGNAL-I-PDU"
            | "CONTAINER-I-PDU"
            | "MULTIPLEXED-I-PDU"
            | "SECURED-I-PDU"
            | "I-SIGNAL"
            | "SYSTEM-SIGNAL"
            | "I-SIGNAL-GROUP"
            | "COMPU-METHOD"
            | "SW-BASE-TYPE"
            | "UNIT"
            | "ECU-INSTANCE"
            | "PDU-TRIGGERING"
            | "TRANSFORMATION-TECHNOLOGY"
            | "DATA-CONSTR"
    )
}

fn object_path(node: Node<'_, '_>) -> String {
    let mut parts = Vec::new();
    for ancestor in node.ancestors() {
        if ancestor.is_element()
            && ancestor.has_tag_name("AR-PACKAGE")
            && let Some(name) = child_text(ancestor, "SHORT-NAME")
        {
            parts.push(name);
        }
    }
    parts.reverse();
    let name = short_name(node);
    if !name.is_empty() {
        parts.push(name);
    }
    format!("/{}", parts.join("/"))
}

fn short_name(node: Node<'_, '_>) -> String {
    child_text(node, "SHORT-NAME").unwrap_or_default()
}

fn child_text(node: Node<'_, '_>, tag: &str) -> Option<String> {
    node.children()
        .find(|child| child.has_tag_name(tag))
        .and_then(|child| child.text())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn text_of(node: Node<'_, '_>, tag: &str) -> Option<String> {
    node.descendants()
        .find(|child| child.has_tag_name(tag))
        .and_then(|child| child.text())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn child_number(node: Node<'_, '_>, tag: &str) -> Option<f64> {
    child_text(node, tag).and_then(|text| text.parse().ok())
}

fn any_number(node: Node<'_, '_>, tag: &str) -> Option<f64> {
    node.descendants()
        .find(|child| child.has_tag_name(tag))
        .and_then(|child| {
            child
                .text()
                .and_then(|text| text.trim().parse().ok())
                .or_else(|| first_number(child))
        })
}

fn child_named_number(node: Node<'_, '_>, tags: &[&str]) -> Option<f64> {
    tags.iter().find_map(|tag| any_number(node, tag))
}

fn first_number(node: Node<'_, '_>) -> Option<f64> {
    node.descendants()
        .filter(|n| n.has_tag_name("VALUE") || n.has_tag_name("V"))
        .find_map(|n| n.text().and_then(|text| text.trim().parse().ok()))
}

fn numbers(node: Node<'_, '_>, tag: &str) -> Vec<f64> {
    node.children()
        .filter(|child| child.has_tag_name(tag))
        .filter_map(|child| child.text().and_then(|text| text.trim().parse().ok()))
        .collect()
}

fn byte_order(node: Node<'_, '_>) -> ByteOrder {
    // R4.2+ matrices put the packing order inside
    // TRANSFORMATION-PROP-SET/DATA-TRANSFORM-PROPS; older files state it
    // directly on the mapping.
    let text = text_of(node, "PACKING-BYTE-ORDER")
        .or_else(|| child_text(node, "BYTE-ORDER"))
        .unwrap_or_default();
    if text.contains("MOST-SIGNIFICANT-BYTE-FIRST")
        || text.contains("BIG-ENDIAN")
        || text == "MOST-SIGNIFICANT-BYTE-FIRST"
    {
        ByteOrder::BigEndian
    } else {
        ByteOrder::LittleEndian
    }
}

fn ref_tail(reference: &str) -> &str {
    reference.rsplit('/').next().unwrap_or(reference)
}

fn desc(node: Node<'_, '_>) -> Option<String> {
    node.descendants()
        .find(|n| n.has_tag_name("L-2"))
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let compact: String = text
        .chars()
        .filter(|ch| !matches!(ch, ':' | '-' | '.' | ' '))
        .collect();
    if compact.len() != 12 || !compact.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let mut mac = [0u8; 6];
    for (index, byte) in mac.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&compact[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(mac)
}
