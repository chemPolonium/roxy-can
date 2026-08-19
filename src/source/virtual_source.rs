use std::f64::consts::PI;

use crate::can::frame::{CanFrame, Direction};
use crate::decode::{from_physical, pack_raw};
use crate::source::FrameSource;

const IDS: [u32; 3] = [0x100, 0x200, 0x320];
const PERIOD_US: [u64; 3] = [10_000, 50_000, 100_000];

pub struct VirtualSource {
    due: [u64; 3],
    counts: [u64; 3],
}

impl VirtualSource {
    pub fn new() -> Self {
        VirtualSource {
            due: PERIOD_US,
            counts: [0; 3],
        }
    }
}

impl Default for VirtualSource {
    fn default() -> Self {
        Self::new()
    }
}

fn encode(data: &mut [u8; 8], start: u64, size: u64, phys: f64, factor: f64, offset: f64) {
    let raw = from_physical(phys, size, false, factor, offset);
    pack_raw(data, start, size, false, raw);
}

impl FrameSource for VirtualSource {
    fn poll(&mut self, now_us: u64, out: &mut Vec<CanFrame>) {
        for i in 0..3 {
            while self.due[i] <= now_us {
                let t = self.due[i];
                let tf = t as f64 / 1e6;
                let mut data = [0u8; 8];
                match IDS[i] {
                    0x100 => {
                        let speed = 3000.0 + 1500.0 * (2.0 * PI * 0.2 * tf).sin();
                        let temp = 90.0 + 10.0 * (2.0 * PI * 0.05 * tf).sin();
                        let throttle = (tf * 7.0) % 100.0;
                        encode(&mut data, 0, 16, speed, 0.25, 0.0);
                        encode(&mut data, 16, 8, temp, 1.0, -40.0);
                        encode(&mut data, 24, 8, throttle, 0.4, 0.0);
                    }
                    0x200 => {
                        let speed = (3000.0 + 1500.0 * (2.0 * PI * 0.2 * tf).sin()) / 37.5;
                        encode(&mut data, 0, 16, speed, 0.01, 0.0);
                        let gear = ((speed / 15.0).floor().clamp(0.0, 5.0) + 1.0) as u64;
                        pack_raw(&mut data, 16, 3, false, gear);
                    }
                    _ => {
                        let pressure = 50.0 + 50.0 * (2.0 * PI * 0.1 * tf).sin().abs();
                        encode(&mut data, 0, 16, pressure, 0.1, 0.0);
                    }
                }
                let dir = if i == 0 && self.counts[i] % 2 == 1 {
                    Direction::Tx
                } else {
                    Direction::Rx
                };
                self.counts[i] += 1;
                out.push(CanFrame {
                    t_us: t,
                    channel: 0,
                    id: IDS[i],
                    extended: false,
                    dlc: 8,
                    data,
                    dir,
                });
                self.due[i] += PERIOD_US[i];
            }
        }
    }
}
