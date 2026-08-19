#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Rx,
    Tx,
}

#[derive(Clone, Copy, Debug)]
pub struct CanFrame {
    pub t_us: u64,
    pub channel: u8,
    pub id: u32,
    pub extended: bool,
    pub dlc: u8,
    pub data: [u8; 8],
    pub dir: Direction,
}
