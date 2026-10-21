//! Automotive Ethernet frames and their trace ring.
//!
//! This is the data-path foundation: frames are kept in their own family,
//! separate from CAN and FlexRay, with a byte cap because a single jumbo
//! frame is far larger than a CAN frame. Decode (ARXML) and UI wiring build
//! on this module.
#![allow(dead_code)]

pub mod db;
pub mod frame;
pub mod ring;

pub use frame::EthFrame;
pub use ring::{EthRing, EthTraceView};
