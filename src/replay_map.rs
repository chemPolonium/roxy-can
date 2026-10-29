//! 回放通道映射：日志里的通道编号 → 本工程的第几路。
//!
//! 为什么需要它：日志每条帧带的只有通道**编号**，而且是录它的那个人工程里
//! 的编号。文件里没有总线名可对照——ASC 的头部只有 `date` / `base` /
//! `version` 三行，BLF 的 CAN 对象只有一个通道号、FlexRay 对象只有一个
//! cluster 号。所以别人的文件对不上自己的路时，能改的只有这层对应关系。
//!
//! 表里没写过的通道按编号原样用，所以从没配过映射的工程读进来行为不变。
//! CAN 一侧编号超出本工程总线数时不回放（见 `BusCore::replay_channel`），
//! 因为"第 3 路"在只有两路的工程里没有任何对应的东西；映射表会把这一路
//! 的条数照列出来，不是悄悄消失。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::ClusterId;

/// 映射属于哪一类总线。FlexRay 的一路和 CAN 的一路是两套编号，
/// 命令、映射表与计数都按这里的顺序区分它们。以太网又是第三套编号：
/// BLF 的以太网对象带的是抓包工具里的 channel，不是 CAN 通道。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogBusKind {
    Can,
    Fr,
    Eth,
}

impl LogBusKind {
    pub fn parse(word: &str) -> Option<Self> {
        match word.to_ascii_lowercase().as_str() {
            "can" => Some(LogBusKind::Can),
            "fr" | "flexray" => Some(LogBusKind::Fr),
            "eth" | "ethernet" => Some(LogBusKind::Eth),
            _ => None,
        }
    }
}

/// 文件通道的落点。没有表项时按文件编号落回本工程的同一路；表项可以
/// 改指一个 cluster，或明确不回放。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayTarget {
    Same,
    Drop,
    Cluster(ClusterId),
}

/// 文件通道 → 工程里的 cluster（不是路号）。cluster 才是工程身份，
/// 运行时由核心解析成具体总线；删/加路不会改写这张表。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplayMap {
    can: BTreeMap<u8, Option<ClusterId>>,
    fr: BTreeMap<u8, Option<ClusterId>>,
    eth: BTreeMap<u8, Option<ClusterId>>,
}

impl ReplayMap {
    /// 文件里的通道 `raw` 落到哪个 cluster；`Same` 表示原编号本路，
    /// `Drop` 表示不回放。
    pub fn target(&self, kind: LogBusKind, raw: u8) -> ReplayTarget {
        match self.table(kind).get(&raw) {
            None => ReplayTarget::Same,
            Some(None) => ReplayTarget::Drop,
            Some(Some(cluster)) => ReplayTarget::Cluster(*cluster),
        }
    }

    /// 记下一条选择；`None` 是明确不回放。
    pub fn set(&mut self, kind: LogBusKind, raw: u8, to: Option<ClusterId>) {
        self.table_mut(kind).insert(raw, to);
    }

    /// 删掉一个 cluster 之后，指向它的表项变成"不回放"；别的表项不动。
    pub fn drop_cluster(&mut self, kind: LogBusKind, cluster: ClusterId) {
        let table = self.table_mut(kind);
        for target in table.values_mut() {
            if *target == Some(cluster) {
                *target = None;
            }
        }
    }

    /// Rebases a v1 map, whose targets are bus indexes, onto the cluster ids
    /// the project now uses. Called only while opening an old project file;
    /// a bus that no longer exists becomes an explicit "don't replay".
    pub fn rebase_buses(&mut self, can: &[u32], fr: &BTreeMap<u8, u32>, eth: &BTreeMap<u8, u32>) {
        let to_cluster = |bus: u8, from_can: bool, from_fr: bool| -> Option<ClusterId> {
            let id = if from_can {
                can.get(bus as usize).copied()
            } else if from_fr {
                fr.get(&bus).copied()
            } else {
                eth.get(&bus).copied()
            };
            id.map(ClusterId)
        };
        for (kind, from_can, from_fr) in [
            (LogBusKind::Can, true, false),
            (LogBusKind::Fr, false, true),
            (LogBusKind::Eth, false, false),
        ] {
            let table = self.table_mut(kind);
            for target in table.values_mut() {
                if let Some(ClusterId(bus)) = *target {
                    *target = to_cluster(bus as u8, from_can, from_fr);
                }
            }
        }
    }

    /// 一条都没配过。工程文件用它决定要不要写这一项。
    pub fn is_empty(&self) -> bool {
        self.can.is_empty() && self.fr.is_empty() && self.eth.is_empty()
    }

    /// 配过的通道（编号升序），目标为 cluster 或 `None`（不回放）。
    pub fn configured(&self, kind: LogBusKind) -> Vec<(u8, Option<ClusterId>)> {
        self.table(kind)
            .iter()
            .map(|(raw, to)| (*raw, *to))
            .collect()
    }

    fn table(&self, kind: LogBusKind) -> &BTreeMap<u8, Option<ClusterId>> {
        match kind {
            LogBusKind::Can => &self.can,
            LogBusKind::Fr => &self.fr,
            LogBusKind::Eth => &self.eth,
        }
    }

    fn table_mut(&mut self, kind: LogBusKind) -> &mut BTreeMap<u8, Option<ClusterId>> {
        match kind {
            LogBusKind::Can => &mut self.can,
            LogBusKind::Fr => &mut self.fr,
            LogBusKind::Eth => &mut self.eth,
        }
    }
}

/// 本次回放从文件里实际见过哪些通道、各多少条，按**文件里的**编号计。
/// 映射表靠它列出"这个文件里有这几路"，于是不必为了数通道把整个文件
/// 再读一遍。数组下标就是编号本身：每帧一次自增，没有查表。
#[derive(Debug, Clone)]
pub struct LogCensus {
    can: [u64; 256],
    fr: [u64; 256],
    eth: [u64; 256],
}

// 数组长度超过 32 时 `Default` 派生用不上，只能自己写零。
impl Default for LogCensus {
    fn default() -> Self {
        LogCensus {
            can: [0; 256],
            fr: [0; 256],
            eth: [0; 256],
        }
    }
}

impl LogCensus {
    pub fn bump(&mut self, kind: LogBusKind, raw: u8) {
        let side = match kind {
            LogBusKind::Can => &mut self.can,
            LogBusKind::Fr => &mut self.fr,
            LogBusKind::Eth => &mut self.eth,
        };
        side[raw as usize] += 1;
    }

    /// 出现过的通道，编号升序；一条都没来过的编号不在里面。
    pub fn rows(&self, kind: LogBusKind) -> Vec<(u8, u64)> {
        let side = match kind {
            LogBusKind::Can => &self.can,
            LogBusKind::Fr => &self.fr,
            LogBusKind::Eth => &self.eth,
        };
        side.iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .map(|(raw, n)| (raw as u8, *n))
            .collect()
    }

    pub fn clear(&mut self) {
        *self = LogCensus::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unlisted_channel_keeps_its_own_number() {
        let map = ReplayMap::default();
        assert_eq!(map.target(LogBusKind::Can, 2), ReplayTarget::Same);
        assert_eq!(map.target(LogBusKind::Fr, 1), ReplayTarget::Same);
    }

    #[test]
    fn a_choice_retargets_and_drops() {
        let mut map = ReplayMap::default();
        map.set(LogBusKind::Can, 3, Some(ClusterId(7)));
        map.set(LogBusKind::Fr, 2, None);
        map.set(LogBusKind::Eth, 1, Some(ClusterId(9)));
        assert_eq!(
            map.target(LogBusKind::Can, 3),
            ReplayTarget::Cluster(ClusterId(7))
        );
        assert_eq!(map.target(LogBusKind::Can, 0), ReplayTarget::Same);
        assert_eq!(map.target(LogBusKind::Fr, 2), ReplayTarget::Drop);
        assert_eq!(
            map.target(LogBusKind::Eth, 1),
            ReplayTarget::Cluster(ClusterId(9))
        );
        assert_eq!(
            map.configured(LogBusKind::Can),
            vec![(3, Some(ClusterId(7)))]
        );
        assert_eq!(map.configured(LogBusKind::Fr), vec![(2, None)]);
        assert_eq!(
            map.configured(LogBusKind::Eth),
            vec![(1, Some(ClusterId(9)))]
        );
    }

    #[test]
    fn deleting_a_cluster_drops_the_rows_that_pointed_at_it() {
        let mut map = ReplayMap::default();
        map.set(LogBusKind::Can, 4, Some(ClusterId(11)));
        map.set(LogBusKind::Can, 5, Some(ClusterId(12)));
        map.set(LogBusKind::Fr, 0, Some(ClusterId(13)));
        map.drop_cluster(LogBusKind::Can, ClusterId(11));
        assert_eq!(
            map.configured(LogBusKind::Can),
            vec![(4, None), (5, Some(ClusterId(12)))]
        );
        assert_eq!(
            map.configured(LogBusKind::Fr),
            vec![(0, Some(ClusterId(13)))]
        );
        assert_eq!(map.target(LogBusKind::Can, 4), ReplayTarget::Drop);
    }

    #[test]
    fn a_v1_map_rebases_bus_numbers_onto_clusters() {
        let mut map = ReplayMap::default();
        map.set(LogBusKind::Can, 1, Some(ClusterId(1)));
        map.set(LogBusKind::Fr, 0, Some(ClusterId(0)));
        map.set(LogBusKind::Eth, 3, Some(ClusterId(3)));
        let fr = [(0u8, 9u32)].into_iter().collect();
        let eth = [(1u8, 5u32)].into_iter().collect();
        map.rebase_buses(&[7, 8], &fr, &eth);
        assert_eq!(
            map.target(LogBusKind::Can, 1),
            ReplayTarget::Cluster(ClusterId(8))
        );
        assert_eq!(
            map.target(LogBusKind::Fr, 0),
            ReplayTarget::Cluster(ClusterId(9))
        );
        assert_eq!(
            map.target(LogBusKind::Eth, 3),
            ReplayTarget::Drop,
            "a bus the project no longer has becomes an explicit drop"
        );
    }

    #[test]
    fn the_census_lists_only_channels_that_arrived_in_number_order() {
        let mut census = LogCensus::default();
        census.bump(LogBusKind::Can, 2);
        census.bump(LogBusKind::Can, 2);
        census.bump(LogBusKind::Can, 0);
        census.bump(LogBusKind::Fr, 1);
        assert_eq!(census.rows(LogBusKind::Can), vec![(0, 1), (2, 2)]);
        assert_eq!(census.rows(LogBusKind::Fr), vec![(1, 1)]);
        assert!(
            !census
                .rows(LogBusKind::Can)
                .iter()
                .any(|(raw, _)| *raw == 1),
            "一条没来过的编号不在表里"
        );
        census.clear();
        assert!(census.rows(LogBusKind::Can).is_empty());
    }

    #[test]
    fn the_map_survives_a_round_trip_through_json() {
        let mut map = ReplayMap::default();
        map.set(LogBusKind::Can, 2, Some(ClusterId(0)));
        map.set(LogBusKind::Fr, 1, None);
        let text = serde_json::to_string(&map).unwrap();
        let back: ReplayMap = serde_json::from_str(&text).unwrap();
        assert_eq!(back, map);
    }

    #[test]
    fn an_empty_table_reads_back_as_no_mapping() {
        let back: ReplayMap = serde_json::from_str("{}").unwrap_or_default();
        assert!(back.configured(LogBusKind::Can).is_empty());
        assert!(back.configured(LogBusKind::Fr).is_empty());
    }
}
