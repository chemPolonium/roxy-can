use super::*;
use serde_json::json;

fn v1() -> crate::config::Config {
    serde_json::from_value(json!({
        "channels": [
            {
                "name": "CAN1",
                "dbc_path": "a.dbc",
                "dbc_paths_extra": ["extra.dbc"],
                "node_roles": { "ECU_A": "Simulated" },
                "bitrate_kbps": 500,
                "fd_data_kbps": 2000
            },
            { "name": "CAN2", "dbc_path": "b.dbc" }
        ],
        "fr_buses": [{ "bus": 1, "path": "fr.xml", "cluster": "FrCluster" }],
        "fr_names": [[1, "Chassis"]],
        "eth_buses": [{ "bus": 0, "path": "eth.arxml", "cluster": "EthCluster" }],
        "eth_frame_length_includes_header": true,
        "can_arxml": [{ "bus": 0, "path": "can.arxml", "cluster": "CanCluster" }],
        "tx": [{ "channel": 1, "id": 256, "data": [1, 2] }],
        "web": { "panelBook": { "pages": [] } }
    }))
    .expect("minimal v1 project parses")
}

#[test]
fn migrates_three_bus_lists_into_clusters() {
    let (project, report) = migrate_v1_clusters(&v1());
    assert!(report.issues.is_empty(), "{:?}", report.issues);
    assert_eq!(project.clusters.len(), 4);
    let names: Vec<&str> = project
        .clusters
        .iter()
        .map(|cluster| cluster.name.as_str())
        .collect();
    assert_eq!(names, vec!["CAN1", "CAN2", "Chassis", "ETH0"]);
    assert_eq!(project.next_cluster_id, 5);

    let can = &project.clusters[0];
    assert_eq!(can.kind, ClusterKind::Can);
    assert_eq!(can.id, 1);
    assert_eq!(
        can.sources,
        vec![
            ClusterSource::Dbc {
                path: "a.dbc".to_string()
            },
            ClusterSource::Dbc {
                path: "extra.dbc".to_string()
            },
            ClusterSource::Arxml {
                path: "can.arxml".to_string(),
                cluster: "CanCluster".to_string()
            }
        ]
    );
    assert_eq!(can.can.as_ref().unwrap().bitrate_kbps, 500);
    assert_eq!(
        can.node_roles.get("ECU_A").map(String::as_str),
        Some("Simulated")
    );

    let fr = &project.clusters[2];
    assert_eq!(fr.kind, ClusterKind::FlexRay);
    assert_eq!(fr.id, 3);
    assert_eq!(
        fr.sources,
        vec![ClusterSource::Fibex {
            path: "fr.xml".to_string()
        }]
    );

    let eth = &project.clusters[3];
    assert_eq!(eth.kind, ClusterKind::Ethernet);
    assert_eq!(eth.id, 4);
    assert_eq!(
        eth.sources,
        vec![ClusterSource::Arxml {
            path: "eth.arxml".to_string(),
            cluster: "EthCluster".to_string()
        }]
    );
    assert!(
        eth.ether
            .as_ref()
            .is_some_and(|ether| ether.frame_length_includes_header)
    );

    assert_eq!(report.legacy_buses.can, vec![ClusterId(1), ClusterId(2)]);
    assert_eq!(
        report.legacy_buses.flexray,
        vec![ClusterId(0), ClusterId(3)]
    );
    assert_eq!(report.legacy_buses.ethernet, vec![ClusterId(4)]);
    assert_eq!(project.legacy_bus_map, Some(report.legacy_buses.clone()));
    assert!(project.validate().is_empty(), "{:?}", project.validate());
}

#[test]
fn migration_keeps_view_sections_and_drops_only_the_bus_lists() {
    let (project, _) = migrate_v1_clusters(&v1());
    assert!(project.rest.contains_key("tx"));
    assert!(project.rest.contains_key("web"));
    for field in [
        "channels",
        "fr_buses",
        "fr_names",
        "eth_buses",
        "can_arxml",
        "eth_frame_length_includes_header",
        "bus_counter",
    ] {
        assert!(!project.rest.contains_key(field), "{field} must be gone");
    }
    let json = serde_json::to_value(&project).unwrap();
    assert_eq!(json["version"], 2);
    assert_eq!(json["tx"][0]["channel"], 1, "view sections stay intact");
}
