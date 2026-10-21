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

/// 映射属于哪一类总线。FlexRay 的一路和 CAN 的一路是两套编号，
/// 命令、映射表与计数都按这里的顺序区分它们。以太网又是第三套编号：
/// BLF 的以太网对象带的是抓包工具里的 channel，不是 CAN 通道。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogBusKind {
    Can,
    Fr,
    Eth,
}

/// 文件通道 → 工程里的一路。值 `None` 表示这一路文件通道不回放。
/// `#[serde(default)]`：工程文件里没有这一项时读成"没配过映射"，
/// 而不是解析失败——旧工程文件正是这样。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplayMap {
    can: BTreeMap<u8, Option<u8>>,
    fr: BTreeMap<u8, Option<u8>>,
    eth: BTreeMap<u8, Option<u8>>,
}

impl ReplayMap {
    /// 文件里的通道 `raw` 落到工程里的哪一路；`None` = 丢掉。
    pub fn target(&self, kind: LogBusKind, raw: u8) -> Option<u8> {
        self.table(kind).get(&raw).copied().unwrap_or(Some(raw))
    }

    /// 日志里第 `raw` 号 CAN 通道回放成本工程的第几路；`None` = 不回放。
    ///
    /// 没配过映射时按编号原样用，但编号在这里必须真对应一条总线：只有两路
    /// 的工程里"第 3 路"什么都不是，那种帧以前以 `CAN3` 的名义进 Trace 又解
    /// 不了码，看着就像总线对上了。挡掉，让映射表把条数说清楚。
    /// 回放和映射表都走这一个函数，两边不会有第二种说法。
    pub fn can_channel(&self, raw: u8, bus_count: usize) -> Option<u8> {
        self.target(LogBusKind::Can, raw)
            .filter(|t| (*t as usize) < bus_count)
    }

    /// 记下一条选择。指回通道自己的编号等于取消这条映射，表里不留着——
    /// 这样以后加了总线，原样用的默认值还能自动生效。
    pub fn set(&mut self, kind: LogBusKind, raw: u8, to: Option<u8>) {
        let table = self.table_mut(kind);
        if to == Some(raw) {
            table.remove(&raw);
        } else {
            table.insert(raw, to);
        }
    }

    /// 工程里的一路没了之后，指到它的那些映射按 `move_target` 改道：新的
    /// 编号，或者 `None`（这一路文件通道不再回放）。留着行而不是悄悄取消，
    /// 是因为映射表会明明白白写出"不回放"，用户看得见自己少了哪几路。
    pub fn shift_targets(
        &mut self,
        kind: LogBusKind,
        mut move_target: impl FnMut(u8) -> Option<u8>,
    ) {
        let table = self.table_mut(kind);
        let moved: BTreeMap<u8, Option<u8>> = table
            .iter()
            .map(|(raw, to)| (*raw, (*to).and_then(&mut move_target)))
            .collect();
        *table = moved;
    }

    /// 一条都没配过。工程文件用它决定要不要写这一项。
    pub fn is_empty(&self) -> bool {
        self.can.is_empty() && self.fr.is_empty() && self.eth.is_empty()
    }

    /// 配过的通道（编号升序）。文件还没回放时，映射表靠它列出已有的选择。
    pub fn configured(&self, kind: LogBusKind) -> Vec<(u8, Option<u8>)> {
        self.table(kind)
            .iter()
            .map(|(raw, to)| (*raw, *to))
            .collect()
    }

    fn table(&self, kind: LogBusKind) -> &BTreeMap<u8, Option<u8>> {
        match kind {
            LogBusKind::Can => &self.can,
            LogBusKind::Fr => &self.fr,
            LogBusKind::Eth => &self.eth,
        }
    }

    fn table_mut(&mut self, kind: LogBusKind) -> &mut BTreeMap<u8, Option<u8>> {
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
        assert_eq!(map.target(LogBusKind::Can, 2), Some(2));
        assert_eq!(map.target(LogBusKind::Fr, 1), Some(1));
    }

    #[test]
    fn a_choice_retargets_and_drops() {
        let mut map = ReplayMap::default();
        map.set(LogBusKind::Can, 3, Some(0));
        map.set(LogBusKind::Fr, 2, None);
        map.set(LogBusKind::Eth, 1, Some(2));
        assert_eq!(map.target(LogBusKind::Can, 3), Some(0));
        assert_eq!(map.target(LogBusKind::Can, 0), Some(0), "untouched");
        assert_eq!(map.target(LogBusKind::Fr, 2), None);
        assert_eq!(
            map.target(LogBusKind::Eth, 1),
            Some(2),
            "a capture channel lands on the project's Ethernet bus"
        );
        assert_eq!(map.configured(LogBusKind::Can), vec![(3, Some(0))]);
        assert_eq!(map.configured(LogBusKind::Fr), vec![(2, None)]);
        assert_eq!(map.configured(LogBusKind::Eth), vec![(1, Some(2))]);
    }

    #[test]
    fn pointing_a_channel_at_itself_forgets_the_row() {
        let mut map = ReplayMap::default();
        map.set(LogBusKind::Can, 1, Some(2));
        map.set(LogBusKind::Can, 1, Some(1));
        assert!(map.configured(LogBusKind::Can).is_empty());
        assert_eq!(map.target(LogBusKind::Can, 1), Some(1));
    }

    #[test]
    fn deleting_a_bus_moves_the_rows_that_pointed_at_it() {
        let mut map = ReplayMap::default();
        map.set(LogBusKind::Can, 4, Some(1));
        map.set(LogBusKind::Can, 5, Some(2));
        map.set(LogBusKind::Fr, 0, Some(1));
        // CAN 删一条，后面的往前挪；被删的那条留下"不回放"，不悄悄取消。
        map.shift_targets(LogBusKind::Can, |c| match c {
            1 => None,
            c if c > 1 => Some(c - 1),
            c => Some(c),
        });
        assert_eq!(
            map.configured(LogBusKind::Can),
            vec![(4, None), (5, Some(1))]
        );
        // FlexRay 的编号不跟着 CAN 挪，各自的表各管自己。
        assert_eq!(map.configured(LogBusKind::Fr), vec![(0, Some(1))]);
        assert_eq!(map.target(LogBusKind::Can, 4), None);
    }

    #[test]
    fn a_channel_with_no_bus_here_has_nowhere_to_go() {
        let map = ReplayMap::default();
        assert_eq!(map.can_channel(0, 2), Some(0));
        assert_eq!(map.can_channel(2, 2), None, "两路的工程里没有第 3 路");
        let mut moved = ReplayMap::default();
        moved.set(LogBusKind::Can, 5, Some(1));
        assert_eq!(
            moved.can_channel(5, 2),
            Some(1),
            "指过来的也过同一道范围检查"
        );
        assert_eq!(moved.can_channel(0, 2), Some(0), "没配的通道不受这条影响");
        moved.set(LogBusKind::Can, 3, None);
        assert_eq!(
            moved.can_channel(3, 8),
            None,
            "选了不回放就是 None，与范围无关"
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
        map.set(LogBusKind::Can, 2, Some(0));
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
