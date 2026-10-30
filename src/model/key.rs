//! Stable bus identity helpers.

use super::ClusterId;

/// Cluster ids by bus index, used to translate schema-v2 rows that still
/// name a bus back to the cluster they belong to.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BusMap {
    pub can: Vec<ClusterId>,
    pub flexray: Vec<ClusterId>,
    pub ethernet: Vec<ClusterId>,
}
