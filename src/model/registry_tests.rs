use super::*;

#[test]
fn registry_imports_multiple_clusters_with_global_ids() {
    let text = "<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>P</SHORT-NAME><ELEMENTS>\
        <CAN-CLUSTER><SHORT-NAME>C</SHORT-NAME></CAN-CLUSTER>\
        <ETHERNET-CLUSTER><SHORT-NAME>E</SHORT-NAME></ETHERNET-CLUSTER>\
        </ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>";
    let mut registry = ClusterRegistry::new();
    let results = registry.add_arxml(text, "multi.arxml");
    assert_eq!(results.len(), 2);
    let ids: Vec<ClusterId> = results.into_iter().map(Result::unwrap).collect();
    assert_eq!(ids, vec![ClusterId(1), ClusterId(2)]);
    assert_eq!(registry.clusters().count(), 2);
    assert_eq!(
        registry.get(ClusterId(1)).unwrap().cfg.kind,
        ClusterKind::Can
    );
    assert_eq!(
        registry.get(ClusterId(2)).unwrap().cfg.kind,
        ClusterKind::Ethernet
    );
    let project = registry.to_project();
    assert_eq!(project.clusters.len(), 2);
    assert_eq!(project.next_cluster_id, 3);
}

#[test]
fn registry_mixes_arxml_dbc_and_fibex() {
    let eth = std::fs::read_to_string(crate::test_assets::eth_arxml()).unwrap();
    let fibex = String::from_utf8_lossy(
        &std::fs::read(crate::test_assets::path("PowerTrain_v2.xml")).unwrap(),
    )
    .into_owned();
    let mut registry = ClusterRegistry::new();
    let eth_id = registry.add_arxml(&eth, "Eth.arxml").remove(0).unwrap();
    let can_id = registry
        .add_dbc(crate::sample_data::MOTBUS_DBC, "motbus.dbc")
        .unwrap();
    let fr_id = registry
        .add_fibex(&fibex, "PowerTrain_v2.xml", false)
        .unwrap();
    assert_eq!(eth_id, ClusterId(1));
    assert_eq!(can_id, ClusterId(2));
    assert_eq!(fr_id, ClusterId(3));
    assert_eq!(registry.clusters().count(), 3);
    assert_eq!(
        registry
            .get(eth_id)
            .unwrap()
            .comm
            .frames
            .values()
            .next()
            .unwrap()
            .name,
        "EthFrame"
    );
    assert_eq!(registry.get(can_id).unwrap().comm.frames.len(), 6);
    assert_eq!(registry.get(fr_id).unwrap().comm.frames.len(), 6);
    let project = registry.to_project();
    assert_eq!(project.clusters.len(), 3);
    assert_eq!(project.next_cluster_id, 4);
    assert!(project.validate().is_empty(), "{:?}", project.validate());
}

fn elements_of(text: &str) -> &str {
    text.split("<ELEMENTS>")
        .nth(1)
        .and_then(|rest| rest.split("</ELEMENTS>").next())
        .expect("fixture has ELEMENTS")
}

#[test]
fn one_arxml_file_can_carry_can_flexray_and_ethernet() {
    let eth = std::fs::read_to_string(crate::test_assets::eth_arxml()).unwrap();
    let text = format!(
        "<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>Multi</SHORT-NAME><ELEMENTS>{}{}{}</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>",
        elements_of(super::arxml_tests::CAN_TEXT),
        elements_of(super::arxml_tests::FR_TEXT),
        elements_of(&eth)
    );
    let mut registry = ClusterRegistry::new();
    let results = registry.add_arxml(&text, "multi.arxml");
    assert_eq!(results.len(), 3, "{results:?}");
    assert!(results.iter().all(Result::is_ok), "{results:?}");
    let kinds: Vec<ClusterKind> = registry
        .clusters()
        .map(|cluster| cluster.cfg.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            ClusterKind::Can,
            ClusterKind::FlexRay,
            ClusterKind::Ethernet
        ]
    );
    assert!(
        registry
            .clusters()
            .all(|cluster| !cluster.comm.frames.is_empty())
    );
}

#[test]
fn project_v2_reopens_with_the_same_clusters_and_ids() {
    let fibex = String::from_utf8_lossy(
        &std::fs::read(crate::test_assets::path("PowerTrain_v2.xml")).unwrap(),
    )
    .into_owned();
    let eth = std::fs::read_to_string(crate::test_assets::eth_arxml()).unwrap();
    let mut registry = ClusterRegistry::new();
    registry
        .add_dbc(
            crate::sample_data::MOTBUS_DBC,
            crate::test_assets::path("motbus.dbc"),
        )
        .unwrap();
    registry
        .add_fibex(&fibex, crate::test_assets::path("PowerTrain_v2.xml"), false)
        .unwrap();
    registry
        .add_arxml(&eth, crate::test_assets::eth_arxml())
        .remove(0)
        .unwrap();
    let project = registry.to_project();
    assert_eq!(project.clusters.len(), 3);

    let (reopened, issues) = ClusterRegistry::from_project(&project, None);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(reopened.clusters().count(), 3);
    let mut frame_counts: Vec<usize> = reopened
        .clusters()
        .map(|entry| entry.comm.frames.len())
        .collect();
    frame_counts.sort_unstable();
    assert_eq!(frame_counts, vec![1, 6, 6]);
    let reproject = reopened.to_project();
    assert_eq!(reproject.next_cluster_id, project.next_cluster_id);
    assert_eq!(
        reproject
            .clusters
            .iter()
            .map(|cluster| cluster.kind)
            .collect::<Vec<_>>(),
        project
            .clusters
            .iter()
            .map(|cluster| cluster.kind)
            .collect::<Vec<_>>()
    );
}

#[test]
fn registry_does_not_import_the_same_source_twice() {
    let mut registry = ClusterRegistry::new();
    registry
        .add_dbc(crate::sample_data::SAMPLE_DBC, "sample.dbc")
        .unwrap();
    let again = registry.add_dbc(crate::sample_data::SAMPLE_DBC, "sample.dbc");
    assert!(again.is_err());
    assert_eq!(registry.clusters().count(), 1);
}
