//! Vector XL Driver runtime binding (vxlapi64.dll / vxlapi.dll, installed
//! with the Vector drivers). Loaded at runtime via LoadLibrary -- no
//! import library, no build-time dependency; a machine without the
//! driver simply gets "no hardware".
//!
//! Signatures and the `XL_CHANNEL_CONFIG` layout follow the vendor
//! header (vxlapi.h, XL_BUS_TYPE_CAN); the struct offsets used for
//! driver-config enumeration were verified with an MSVC `offsetof`
//! probe against that header, not assumed:
//! `sizeof(XL_CHANNEL_CONFIG) = 227` (the struct is packed),
//! `name` at 0, `channelIndex` at 41, `channelMask` at 42,
//! `sizeof(XLdriverConfig) = 14576`, `channelCount` at 4,
//! `channel[0]` at 48, `sizeof(XLcanTxEvent) = 88`,
//! `sizeof(XLcanRxEvent) = 128`, `data` at 64 inside `canRxOkMsg`.
//!
//! Safety surface: the loaded function pointers are C-callable exports
//! only ever invoked through the safe wrapper below. Handles are owned
//! by [`VectorChannel`], which closes them on drop.
//!
//! First cut scope: classic CAN. CAN FD on Vector goes through the
//! `XLcanFdConf` parameter struct (a separate, larger FFI surface) and
//! is deferred -- `fd` is always false here, matching the Kvaser
//! receive-only degrade path.

use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN};
use std::ffi::c_void;

type XlStatus = i16;
type XlPortHandle = i32;
type XlAccess = u64;

const XL_BUS_TYPE_CAN: u32 = 1;
const XL_INTERFACE_VERSION_V4: u32 = 4;
/// xlReceive/xlCanReceive report this when the queue is drained.
const XL_ERR_QUEUE_IS_EMPTY: XlStatus = 10;
/// A failed xlOpenPort leaves this in the port handle (verified live:
/// a successful open returned 0 and 1 -- 0 is a valid handle).
const XL_INVALID_PORT: XlPortHandle = -1;
/// The driver config buffer: 48-byte header + 64 packed channel records.
const DRIVER_CONFIG_SIZE: usize = 14_576;
const DRIVER_CONFIG_CHANNEL_STRIDE: usize = 227;
const DRIVER_CONFIG_CHANNEL_COUNT: usize = 4;
const DRIVER_CONFIG_CHANNELS: usize = 48;
/// Event tags (XLcanRxEvent.tag).
const XL_CAN_EV_TAG_RX_OK: u16 = 0x0400;
/// Message flags shared by RX and TX events.
const XL_CAN_MSG_FLAG_EDL: u32 = 0x0001;
const XL_CAN_MSG_FLAG_BRS: u32 = 0x0002;
const XL_CAN_MSG_FLAG_ESI: u32 = 0x0004;
const XL_CAN_MSG_FLAG_RTR: u32 = 0x0010;
/// The extended-frame flag lives in bit 31 of the CAN id itself.
const CAN_ID_EXTENDED: u32 = 0x8000_0000;
/// RX event layout (verified offsets inside the 128-byte event).
const RX_EVENT_CAN_ID: usize = 32;
const RX_EVENT_MSG_FLAGS: usize = 36;
const RX_EVENT_DLC: usize = 58;
const RX_EVENT_DATA: usize = 64;
/// TX event layout (verified offsets inside the 88-byte event).
const TX_EVENT_CAN_ID: usize = 8;
const TX_EVENT_MSG_FLAGS: usize = 12;
const TX_EVENT_DLC: usize = 16;
const TX_EVENT_DATA: usize = 24;

type XlOpenDriver = unsafe extern "system" fn() -> XlStatus;
type XlCloseDriver = unsafe extern "system" fn() -> XlStatus;
type XlGetDriverConfig = unsafe extern "system" fn(*mut u8) -> XlStatus;
type XlOpenPort = unsafe extern "system" fn(
    *mut XlPortHandle,
    *const u8,
    XlAccess,
    *mut XlAccess,
    u32,
    u32,
    u32,
) -> XlStatus;
type XlCanSetChannelBitrate = unsafe extern "system" fn(XlPortHandle, XlAccess, u32) -> XlStatus;
type XlActivateChannel =
    unsafe extern "system" fn(XlPortHandle, XlAccess, u32, u32) -> XlStatus;
type XlDeactivateChannel = unsafe extern "system" fn(XlPortHandle, XlAccess) -> XlStatus;
type XlCanReceive = unsafe extern "system" fn(XlPortHandle, *mut u8) -> XlStatus;
type XlCanTransmitEx =
    unsafe extern "system" fn(XlPortHandle, XlAccess, u32, *mut u32, *mut u8) -> XlStatus;
type XlFlushReceiveQueue = unsafe extern "system" fn(XlPortHandle) -> XlStatus;
type XlClosePort = unsafe extern "system" fn(XlPortHandle) -> XlStatus;
type XlGetErrorString = unsafe extern "system" fn(XlStatus) -> *const u8;
type XlGetApplConfig = unsafe extern "system" fn(
    *const u8,
    u32,
    *mut u32,
    *mut u32,
    *mut u32,
    u32,
) -> XlStatus;
type XlSetApplConfig = unsafe extern "system" fn(
    *const u8,
    u32,
    u32,
    u32,
    u32,
    u32,
) -> XlStatus;
/// Returns the channel mask directly (0 = not found), not a status.
type XlGetChannelIndex = unsafe extern "system" fn(u32, u32, u32) -> XlAccess;

/// Every vxlapi entry the tool needs. `None` members mean the export was
/// missing -- the wrapper reports "driver not available".
#[derive(Debug)]
struct Vxlapi {
    open_driver: XlOpenDriver,
    close_driver: XlCloseDriver,
    get_driver_config: XlGetDriverConfig,
    open_port: XlOpenPort,
    can_set_channel_bitrate: XlCanSetChannelBitrate,
    activate_channel: XlActivateChannel,
    deactivate_channel: XlDeactivateChannel,
    can_receive: XlCanReceive,
    can_transmit_ex: XlCanTransmitEx,
    flush_receive_queue: XlFlushReceiveQueue,
    close_port: XlClosePort,
    /// Serves driver-error text in attach failures.
    get_error_string: XlGetErrorString,
    /// The application's registered channel mapping; registration via
    /// `set_appl_config` on first use.
    #[allow(dead_code)]
    get_appl_config: XlGetApplConfig,
    #[allow(dead_code)]
    set_appl_config: XlSetApplConfig,
    #[allow(dead_code)]
    get_channel_index: XlGetChannelIndex,
}

impl Vxlapi {
    /// Loads the 64-bit driver DLL first, then the generic name (which
    /// also resolves to a 64-bit copy on x64 via the System32 search).
    unsafe fn load() -> Option<Self> {
        const NAMES: [&[u16]; 2] = [
            // vxlapi64.dll — the x64 driver DLL.
            &[
                b'v' as u16, b'x' as u16, b'l' as u16, b'a' as u16, b'p' as u16, b'i' as u16,
                b'6' as u16, b'4' as u16, b'.' as u16, b'd' as u16, b'l' as u16, b'l' as u16, 0,
            ],
            // vxlapi.dll fallback.
            &[
                b'v' as u16, b'x' as u16, b'l' as u16, b'a' as u16, b'p' as u16, b'i' as u16,
                b'.' as u16, b'd' as u16, b'l' as u16, b'l' as u16, 0,
            ],
        ];
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn LoadLibraryW(name: *const u16) -> *mut c_void;
            fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        }
        let mut module: *mut c_void = std::ptr::null_mut();
        for name in NAMES {
            // SAFETY: the name is a valid NUL-terminated wide string; loading
            // a system-installed DLL has no side effects on our state.
            module = unsafe { LoadLibraryW(name.as_ptr()) };
            if !module.is_null() {
                break;
            }
        }
        if module.is_null() {
            return None;
        }
        let sym = |name: &[u8]| -> *mut c_void {
            let mut bytes = name.to_vec();
            bytes.push(0);
            // SAFETY: module is a live handle from LoadLibraryW above and
            // the name is NUL-terminated.
            unsafe { GetProcAddress(module, bytes.as_ptr()) }
        };
        macro_rules! need {
            ($name:expr, $ty:ty) => {
                // SAFETY: the pointer came from GetProcAddress for exactly
                // this symbol name, cast to its declared signature.
                match unsafe { std::mem::transmute::<*mut c_void, Option<$ty>>(sym($name)) } {
                    Some(f) => f,
                    None => return None,
                }
            };
        }
        Some(Vxlapi {
            open_driver: need!(b"xlOpenDriver", XlOpenDriver),
            close_driver: need!(b"xlCloseDriver", XlCloseDriver),
            get_driver_config: need!(b"xlGetDriverConfig", XlGetDriverConfig),
            open_port: need!(b"xlOpenPort", XlOpenPort),
            can_set_channel_bitrate: need!(b"xlCanSetChannelBitrate", XlCanSetChannelBitrate),
            activate_channel: need!(b"xlActivateChannel", XlActivateChannel),
            deactivate_channel: need!(b"xlDeactivateChannel", XlDeactivateChannel),
            can_receive: need!(b"xlCanReceive", XlCanReceive),
            can_transmit_ex: need!(b"xlCanTransmitEx", XlCanTransmitEx),
            flush_receive_queue: need!(b"xlFlushReceiveQueue", XlFlushReceiveQueue),
            close_port: need!(b"xlClosePort", XlClosePort),
            get_error_string: need!(b"xlGetErrorString", XlGetErrorString),
            get_appl_config: need!(b"xlGetApplConfig", XlGetApplConfig),
            set_appl_config: need!(b"xlSetApplConfig", XlSetApplConfig),
            get_channel_index: need!(b"xlGetChannelIndex", XlGetChannelIndex),
        })
    }

    /// The driver's own error text for a status code (static buffer).
    /// Every failure that reaches the user carries this, not just the
    /// number -- "status 101" alone has cost an entire debugging round.
    fn error_text(&self, status: XlStatus) -> String {
        // SAFETY: the export returns a pointer to a driver-owned static
        // string valid for the process lifetime.
        let raw = unsafe { (self.get_error_string)(status) };
        if raw.is_null() {
            return String::new();
        }
        // SAFETY: NUL-terminated ANSI text from the driver.
        let bytes = unsafe { std::ffi::CStr::from_ptr(raw as *const std::ffi::c_char) };
        bytes.to_string_lossy().into_owned()
    }

    /// `status {code} ({text})` for failure messages.
    fn error(&self, status: XlStatus) -> String {
        let text = self.error_text(status);
        if text.is_empty() {
            format!("status {status}")
        } else {
            format!("status {status}: {text}")
        }
    }

    fn lib() -> Option<&'static Vxlapi> {
        static LIB: std::sync::OnceLock<Option<Vxlapi>> = std::sync::OnceLock::new();
        LIB.get_or_init(|| unsafe { Vxlapi::load() })
            .as_ref()
    }
}

/// One discoverable Vector channel: the driver's global channel index
/// plus the hardware name from the driver config.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelInfo {
    pub index: i32,
    pub name: String,
}

/// An open, activated Vector port for one CAN channel. Frames written
/// here leave on the wire; `try_read` drains the driver's receive queue.
#[derive(Debug)]
pub struct VectorChannel {
    port: XlPortHandle,
    /// The single-channel mask this port was opened for.
    mask: XlAccess,
    /// FD data-phase params were applied; FD frames can leave. Always
    /// false in the first cut (see module docs).
    pub fd: bool,
}

/// CAN FD data length from the DLC + EDL/RTR flags
/// (the vendor's `CANFD_GET_NUM_DATABYTES`).
fn fd_num_data_bytes(dlc: u8, edl: bool, rtr: bool) -> usize {
    if rtr {
        0
    } else if dlc < 9 {
        dlc as usize
    } else if !edl {
        8
    } else {
        match dlc {
            9 => 12,
            10 => 16,
            11 => 20,
            12 => 24,
            13 => 32,
            14 => 48,
            _ => 64,
        }
    }
}

/// The DLC byte that carries `len` payload bytes: direct up to 8, then
/// the CAN FD length ladder (the inverse of [`fd_num_data_bytes`]).
fn dlc_from_len(len: usize, fd: bool) -> u8 {
    if len <= 8 {
        return len as u8;
    }
    if !fd {
        return 8;
    }
    match len {
        12 => 9,
        16 => 10,
        20 => 11,
        24 => 12,
        32 => 13,
        48 => 14,
        _ => 15,
    }
}

impl VectorChannel {
    /// Opens the channel (global channel index from the driver config)
    /// and activates it at the given arbitration bitrate.
    ///
    /// `init_access = false` opens without init permission: it succeeds
    /// even when another program holds the channel -- the monitoring
    /// path. `fd_data_kbps` is accepted for interface parity with the
    /// Kvaser path but not yet applied (Vector FD config goes through
    /// `XLcanFdConf`, deferred); `fd` therefore stays false.
    ///
    /// The permission request names the channels in the mask, not the
    /// whole 64-bit space (`u64::MAX` reads as an out-of-range channel
    /// and fails with `XL_ERR_WRONG_PARAMETER`); the driver downgrades
    /// to basic access when another program holds the channel, which
    /// the init path reports so the attach logic can retry rx-only.
    pub fn open(
        index: i32,
        kbps: u32,
        fd_data_kbps: Option<u32>,
        init_access: bool,
    ) -> Result<VectorChannel, String> {
        let _ = fd_data_kbps;
        let lib = Vxlapi::lib().ok_or("Vector 驱动不可用（vxlapi64.dll / vxlapi.dll 未找到）")?;
        unsafe {
            let status = (lib.open_driver)();
            if status != 0 {
                return Err(format!("xlOpenDriver 失败（{}）", lib.error(status)));
            }
            let mask = 1u64 << index;
            let mut permission: XlAccess = if init_access { mask } else { 0 };
            let mut port: XlPortHandle = 0;
            let status = (lib.open_port)(
                &mut port,
                c"roxy-can".as_ptr() as *const u8,
                mask,
                &mut permission,
                65_536,
                XL_INTERFACE_VERSION_V4,
                XL_BUS_TYPE_CAN,
            );
            if status != 0 || port == XL_INVALID_PORT {
                (lib.close_driver)();
                return Err(format!(
                    "打开 Vector 通道 {index} 失败（{}）",
                    lib.error(status)
                ));
            }
            let granted = permission & mask != 0;
            if init_access && !granted {
                // Downgraded to basic access: another program owns the
                // channel. Close and let the attach logic retry rx-only.
                (lib.close_port)(port);
                (lib.close_driver)();
                return Err("通道被其他程序占用（只得到只收权限）".to_string());
            }
            // Bitrate needs init access; a monitoring port attaches at
            // whatever the bus already runs.
            if init_access {
                let bitrate = kbps.saturating_mul(1_000);
                let status = (lib.can_set_channel_bitrate)(port, mask, bitrate);
                if status != 0 {
                    (lib.close_port)(port);
                    (lib.close_driver)();
                    return Err(format!(
                        "设置波特率 {kbps} kbit/s 失败（{}）",
                        lib.error(status)
                    ));
                }
            }
            let status = (lib.activate_channel)(port, mask, XL_BUS_TYPE_CAN, 0);
            if status != 0 {
                (lib.close_port)(port);
                (lib.close_driver)();
                return Err(format!("激活通道失败（{}）", lib.error(status)));
            }
            // Frames queued while parked (or from a previous owner of the
            // channel) must not leak into this run's views.
            (lib.flush_receive_queue)(port);
            Ok(VectorChannel {
                port,
                mask,
                fd: false,
            })
        }
    }

    /// Writes one frame out. Ids over 0x7FF go extended (bit 31 of the
    /// CAN id); RTR frames keep their flag and carry no payload; FD
    /// frames carry the EDL/BRS flags (they fail on a channel whose
    /// Vector configuration has no FD data-phase params).
    pub fn write_frame(&self, f: &CanFrame) -> Result<(), String> {
        let lib = Vxlapi::lib().ok_or("Vector 驱动不可用")?;
        let payload = if f.is_remote() { &[][..] } else { f.payload() };
        let len = payload.len().min(MAX_CAN_FD_LEN);
        let mut tx = [0u8; 88];
        tx[0] = 0x40; // XL_CAN_EV_TAG_TX_MSG = 0x0440 (little-endian)
        tx[1] = 0x04;
        // transId + channelIndex + reserved stay zero.
        let can_id = f.id | if f.extended { CAN_ID_EXTENDED } else { 0 };
        tx[TX_EVENT_CAN_ID..TX_EVENT_CAN_ID + 4]
            .copy_from_slice(&can_id.to_le_bytes());
        let mut msg_flags = 0u32;
        if f.is_remote() {
            msg_flags |= XL_CAN_MSG_FLAG_RTR;
        }
        if f.is_fd() {
            msg_flags |= XL_CAN_MSG_FLAG_EDL;
            if f.flags.contains(FrameFlags::BRS) {
                msg_flags |= XL_CAN_MSG_FLAG_BRS;
            }
            if f.flags.contains(FrameFlags::ESI) {
                msg_flags |= XL_CAN_MSG_FLAG_ESI;
            }
        }
        tx[TX_EVENT_MSG_FLAGS..TX_EVENT_MSG_FLAGS + 4]
            .copy_from_slice(&msg_flags.to_le_bytes());
        // DLC encodes the payload length: direct up to 8, then the CAN FD
        // length ladder (12/16/20/24/32/48/64 bytes).
        tx[TX_EVENT_DLC] = dlc_from_len(len, f.is_fd());
        let payload = if f.is_remote() { &[][..] } else { f.payload() };
        let n = payload.len().min(64);
        tx[TX_EVENT_DATA..TX_EVENT_DATA + n].copy_from_slice(&payload[..n]);

        let mut sent: u32 = 0;
        let status = unsafe {
            (lib.can_transmit_ex)(self.port, self.mask, 1, &mut sent, tx.as_mut_ptr())
        };
        if status == 0 && sent == 1 {
            Ok(())
        } else if status == 0 {
            Err("帧未进入发送队列".to_string())
        } else {
            Err(format!("xlCanTransmitEx 失败（{}）", lib.error(status)))
        }
    }

    /// Takes one received frame off the queue, if any. Non-frame events
    /// (chip state, TX acknowledgements) are skipped internally; `None`
    /// when the queue is empty or the read failed (a failed bus reports
    /// itself on the next write anyway).
    pub fn try_read(&mut self) -> Option<CanFrame> {
        let lib = Vxlapi::lib()?;
        loop {
            let mut ev = [0u8; 128];
            let status = unsafe { (lib.can_receive)(self.port, ev.as_mut_ptr()) };
            if status == XL_ERR_QUEUE_IS_EMPTY {
                return None;
            }
            if status != 0 {
                return None;
            }
            let tag = u16::from_le_bytes([ev[4], ev[5]]);
            if tag != XL_CAN_EV_TAG_RX_OK {
                continue; // chip state / TX ack / error events: not frames
            }
            let raw_id = u32::from_le_bytes(ev[RX_EVENT_CAN_ID..RX_EVENT_CAN_ID + 4].try_into().ok()?);
            let msg_flags = u32::from_le_bytes(ev[RX_EVENT_MSG_FLAGS..RX_EVENT_MSG_FLAGS + 4].try_into().ok()?);
            let extended = raw_id & CAN_ID_EXTENDED != 0;
            let id = raw_id & if extended { 0x1FFF_FFFF } else { 0x7FF };
            let is_fd = msg_flags & XL_CAN_MSG_FLAG_EDL != 0;
            let is_rtr = msg_flags & XL_CAN_MSG_FLAG_RTR != 0;
            let is_brs = msg_flags & XL_CAN_MSG_FLAG_BRS != 0;
            let is_esi = msg_flags & XL_CAN_MSG_FLAG_ESI != 0;
            let dlc = ev[RX_EVENT_DLC];
            let len = fd_num_data_bytes(dlc, is_fd, is_rtr).min(MAX_CAN_FD_LEN) as u8;
            let mut data = [0u8; MAX_CAN_FD_LEN];
            data[..len as usize]
                .copy_from_slice(&ev[RX_EVENT_DATA..RX_EVENT_DATA + len as usize]);
            let mut flags = FrameFlags::NONE;
            if is_fd {
                flags = flags.union(FrameFlags::FD);
            }
            if is_brs {
                flags = flags.union(FrameFlags::BRS);
            }
            if is_esi {
                flags = flags.union(FrameFlags::ESI);
            }
            if is_rtr {
                flags = flags.union(FrameFlags::RTR);
            }
            return Some(CanFrame {
                t_us: 0, // stamped by the core against the sim clock
                channel: 0,
                id,
                extended,
                len,
                data,
                dir: Direction::Rx,
                flags,
            });
        }
    }
}

impl Drop for VectorChannel {
    fn drop(&mut self) {
        if let Some(lib) = Vxlapi::lib() {
            unsafe {
                (lib.deactivate_channel)(self.port, self.mask);
                (lib.close_port)(self.port);
                (lib.close_driver)();
            }
        }
    }
}

/// Enumerates every channel the installed Vector driver knows about,
/// named from the driver config. An empty result is a valid answer
/// (driver present, no adapters); `Err` means the driver itself is
/// unavailable.
pub fn enumerate() -> Result<Vec<ChannelInfo>, String> {
    let lib = Vxlapi::lib().ok_or_else(|| {
        "Vector 驱动不可用（vxlapi64.dll / vxlapi.dll 未找到）".to_string()
    })?;
    unsafe {
        let status = (lib.open_driver)();
        if status != 0 {
            return Err(format!("xlOpenDriver 失败（{}）", lib.error(status)));
        }
        let mut config = [0u8; DRIVER_CONFIG_SIZE];
        let status = (lib.get_driver_config)(config.as_mut_ptr());
        if status != 0 {
            (lib.close_driver)();
            return Err(format!("xlGetDriverConfig 失败（{}）", lib.error(status)));
        }
        let count = u32::from_le_bytes(
            config[DRIVER_CONFIG_CHANNEL_COUNT..DRIVER_CONFIG_CHANNEL_COUNT + 4]
                .try_into()
                .unwrap(),
        )
        .min(64) as usize;
        let mut out = Vec::new();
        for i in 0..count {
            let rec = DRIVER_CONFIG_CHANNELS + i * DRIVER_CONFIG_CHANNEL_STRIDE;
            let name_bytes = &config[rec..rec + 32];
            let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(32);
            let name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();
            let channel_index = config[rec + 41];
            out.push(ChannelInfo {
                index: channel_index as i32,
                name: if name.is_empty() {
                    format!("Vector 通道 {channel_index}")
                } else {
                    name
                },
            });
        }
        (lib.close_driver)();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live probe against the machine's real vxlapi driver: dump every
    /// channel's config fields, then run the fixed open path end to end
    /// -- init open, bitrate, activate -- on the first virtual channel,
    /// plus an rx-only open of the next, and prove the pair passes a
    /// frame (the virtual bus loops transmit back as receive). Leaves
    /// the driver's app-config registration for "roxy-can" pointing at
    /// the first virtual channel (it started life as junk from an early
    /// probe).
    #[test]
    fn vector_open_probe_loops_a_frame_over_the_virtual_bus() {
        let Some(lib) = Vxlapi::lib() else {
            println!("no vxlapi -- probe skipped");
            return;
        };
        let channels = enumerate().expect("enumerate");
        println!("driver channels: {channels:?}");
        if channels.len() < 2 {
            println!("need two virtual channels for the loop -- probe skipped");
            return;
        }
        unsafe {
            let status = (lib.open_driver)();
            assert_eq!(status, 0, "xlOpenDriver: {}", lib.error(status));
            // Registration housekeeping: point app channel 0 at the first
            // virtual channel (hwType 1 = XL_HWTYPE_VIRTUAL).
            let (vch0, vch1) = (channels[0].index, channels[1].index);
            let status = (lib.set_appl_config)(
                c"roxy-can".as_ptr() as *const u8,
                0,
                1,
                0x160_000,
                0,
                XL_BUS_TYPE_CAN,
            );
            println!("SetApplConfig housekeeping: {status}");

            // The production init path on channel A.
            let a = VectorChannel::open(vch0, 500, None, true).expect("init open");
            assert!(!a.fd);
            // The production monitoring path on channel B.
            let mut b = VectorChannel::open(vch1, 500, None, false).expect("rx open");

            // Transmit a frame on A; the virtual bus delivers it to B.
            let frame = CanFrame {
                t_us: 0,
                channel: 0,
                id: 0x123,
                extended: false,
                len: 2,
                data: {
                    let mut d = [0u8; MAX_CAN_FD_LEN];
                    d[..2].copy_from_slice(&[0xAB, 0xCD]);
                    d
                },
                dir: Direction::Tx,
                flags: FrameFlags::NONE,
            };
            a.write_frame(&frame).expect("tx");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let got = loop {
                if std::time::Instant::now() > deadline {
                    panic!("the looped frame never arrived on the second channel");
                }
                if let Some(f) = b.try_read() {
                    break f;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            assert_eq!((got.id, got.len, got.data[0]), (0x123, 2, 0xAB));
            println!("loopback ok: id=0x{:X} len={} data[0]=0x{:02X}", got.id, got.len, got.data[0]);
        }
    }
}
