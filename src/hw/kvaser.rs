//! Kvaser CANlib runtime binding. The vendor DLL (canlib32.dll, installed
//! with the Kvaser drivers) is loaded at runtime via LoadLibrary -- no
//! import library, no build-time dependency; a machine without the
//! driver simply gets "no hardware".
//!
//! Safety surface: the loaded function pointers are stdcall and only
//! ever called through the safe wrapper below. Handles are owned by
//! [`KvaserChannel`], which closes them on drop.

use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN};
use std::ffi::c_void;

type CanStatus = i32;
type CanHandle = i32;

pub const CAN_OK: i32 = 0;

const CAN_MSG_STD: u32 = 0x0001;
const CAN_MSG_EXT: u32 = 0x0004;
const CAN_MSG_RTR: u32 = 0x0002;
const CAN_FDMSG: u32 = 0x0080;

/// Kvaser preset bitrate codes (negative = table entry; the tseg
/// arguments are ignored). Keyed by our kbit/s values.
fn bitrate_code(kbps: u32) -> i32 {
    match kbps {
        1000 => -1,
        500 => -4,
        250 => -8,
        125 => -9,
        100 => -10,
        83 => -11,
        62 => -12,
        50 => -13,
        10 => -15,
        _ => -4, // the de-facto default
    }
}

type CanInitializeLibrary = unsafe extern "system" fn() -> CanStatus;
type CanGetNumberOfChannels = unsafe extern "system" fn(*mut i32) -> CanStatus;
type CanOpenChannel = unsafe extern "system" fn(i32, i32) -> CanHandle;
type CanSetBusParams =
    unsafe extern "system" fn(i32, i32, u8, u8, u8, u8, u32) -> CanStatus;
type CanBusOn = unsafe extern "system" fn(i32) -> CanStatus;
type CanBusOff = unsafe extern "system" fn(i32) -> CanStatus;
type CanWrite = unsafe extern "system" fn(i32, u32, *const u8, u32, u32) -> CanStatus;
type CanRead =
    unsafe extern "system" fn(i32, *mut u32, *mut u8, *mut u32, *mut u32, *mut u32) -> CanStatus;
type CanClose = unsafe extern "system" fn(i32) -> CanStatus;

/// Every canlib entry the tool needs. `None` members mean the DLL or the
/// export was missing -- the wrapper reports "driver not available".
#[derive(Debug)]
struct Canlib {
    initialize_library: CanInitializeLibrary,
    get_number_of_channels: CanGetNumberOfChannels,
    open_channel: CanOpenChannel,
    set_bus_params: CanSetBusParams,
    bus_on: CanBusOn,
    bus_off: CanBusOff,
    write: CanWrite,
    read: CanRead,
    close: CanClose,
}

impl Canlib {
    unsafe fn load() -> Option<Self> {
        const LOAD_LIBRARY_W_NAME: &[u16] = &[
            b'c' as u16, b'a' as u16, b'n' as u16, b'l' as u16, b'i' as u16, b'b' as u16,
            b'3' as u16, b'2' as u16, b'.' as u16, b'd' as u16, b'l' as u16, b'l' as u16, 0,
        ];
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn LoadLibraryW(name: *const u16) -> *mut c_void;
            fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        }
        // SAFETY: the name is a valid NUL-terminated wide string; loading
        // a system-installed DLL has no side effects on our state.
        let module = unsafe { LoadLibraryW(LOAD_LIBRARY_W_NAME.as_ptr()) };
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
        Some(Canlib {
            initialize_library: need!(b"canInitializeLibrary", CanInitializeLibrary),
            get_number_of_channels: need!(b"canGetNumberOfChannels", CanGetNumberOfChannels),
            open_channel: need!(b"canOpenChannel", CanOpenChannel),
            set_bus_params: need!(b"canSetBusParams", CanSetBusParams),
            bus_on: need!(b"canBusOn", CanBusOn),
            bus_off: need!(b"canBusOff", CanBusOff),
            write: need!(b"canWrite", CanWrite),
            read: need!(b"canRead", CanRead),
            close: need!(b"canClose", CanClose),
        })
    }

    fn lib() -> Option<&'static Canlib> {
        static LIB: std::sync::OnceLock<Option<Canlib>> = std::sync::OnceLock::new();
        LIB.get_or_init(|| unsafe { Canlib::load() })
            .as_ref()
    }
}

/// One discoverable Kvaser channel.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelInfo {
    pub index: i32,
    pub name: String,
}

/// An open, bus-on channel. Frames written here leave on the wire
/// (physical channel) or loop into the driver (virtual channel).
#[derive(Debug)]
pub struct KvaserChannel {
    handle: CanHandle,
}

impl KvaserChannel {
    /// Opens a channel and puts the bus on at the given bitrate.
    pub fn open(index: i32, kbps: u32) -> Result<KvaserChannel, String> {
        let lib = Canlib::lib().ok_or("Kvaser 驱动不可用（canlib32.dll 未找到）")?;
        unsafe {
            (lib.initialize_library)();
            // flags 0 = 默认共享打开；不带 ACCEPT_VIRTUAL 位——它在这部分
            // 驱动上会得到 canERR_PARAM。物理通道直接可开。
            let handle = (lib.open_channel)(index, 0);
            if handle < 0 {
                return Err(format!("打开 Kvaser 通道 {index} 失败（status {handle}）"));
            }
            let code = bitrate_code(kbps);
            let status = (lib.set_bus_params)(handle, code, 0, 0, 0, 0, 0);
            if status != CAN_OK {
                (lib.close)(handle);
                return Err(format!("设置波特率失败（status {status}）"));
            }
            let status = (lib.bus_on)(handle);
            if status != CAN_OK {
                (lib.close)(handle);
                return Err(format!("BusOn 失败（status {status}）"));
            }
            Ok(KvaserChannel { handle })
        }
    }

    /// Writes one frame out. Ids over 0x7FF go extended; RTR frames keep
    /// their flag and carry no payload. FD frames (payload up to 64
    /// bytes) go out with the canFDMSG marker.
    pub fn write_frame(&self, f: &CanFrame) -> Result<(), String> {
        let lib = Canlib::lib().ok_or("Kvaser 驱动不可用")?;
        let len = f.payload().len().min(MAX_CAN_FD_LEN);
        let mut flag = if f.extended {
            CAN_MSG_EXT
        } else if f.is_remote() {
            CAN_MSG_RTR
        } else {
            CAN_MSG_STD
        };
        if f.is_fd() {
            flag |= CAN_FDMSG;
        }
        let status = unsafe {
            (lib.write)(self.handle, f.id, f.payload().as_ptr(), len as u32, flag)
        };
        if status == CAN_OK {
            Ok(())
        } else {
            Err(format!("canWrite 失败（status {status}）"))
        }
    }

    /// Takes one received frame off the queue, if any. `None` when the
    /// queue is empty or the read failed (a failed bus reports itself on
    /// the next write anyway).
    pub fn try_read(&mut self) -> Option<CanFrame> {
        let lib = Canlib::lib()?;
        let mut id: u32 = 0;
        let mut data = [0u8; MAX_CAN_FD_LEN];
        let mut dlc: u32 = 0;
        let mut flag: u32 = 0;
        let mut time: u32 = 0;
        let status = unsafe {
            (lib.read)(
                self.handle,
                &mut id,
                data.as_mut_ptr(),
                &mut dlc,
                &mut flag,
                &mut time,
            )
        };
        if status != CAN_OK {
            return None;
        }
        let extended = flag & CAN_MSG_EXT != 0;
        let is_remote = flag & CAN_MSG_RTR != 0;
        let is_error = flag & 0x0008 != 0;
        let len = dlc.min(MAX_CAN_FD_LEN as u32) as u8;
        let mut flags = FrameFlags::NONE;
        if flag & 0x0080 != 0 {
            flags = flags.union(FrameFlags::FD);
        }
        if is_error {
            flags = flags.union(FrameFlags::ERROR);
        }
        if is_remote {
            flags = flags.union(FrameFlags::RTR);
        }
        Some(CanFrame {
            t_us: 0, // stamped by the core against the sim clock
            channel: 0,
            id,
            extended,
            len,
            data,
            dir: Direction::Rx,
            flags,
        })
    }
}

impl Drop for KvaserChannel {
    fn drop(&mut self) {
        if let Some(lib) = Canlib::lib() {
            unsafe {
                (lib.bus_off)(self.handle);
                (lib.close)(self.handle);
            }
        }
    }
}

/// Enumerates every channel the installed drivers know about. An empty
/// result is a valid answer (driver present, no adapters); `Err` means
/// the driver itself is unavailable.
pub fn enumerate() -> Result<Vec<ChannelInfo>, String> {
    let lib = Canlib::lib().ok_or_else(|| "Kvaser 驱动不可用（canlib32.dll 未找到）".to_string())?;
    unsafe {
        (lib.initialize_library)();
        let mut num: i32 = 0;
        let status = (lib.get_number_of_channels)(&mut num);
        if status != CAN_OK {
            return Err(format!("canGetNumberOfChannels 失败（status {status}）"));
        }
        let mut out = Vec::new();
        for index in 0..num {
            // The driver's name-lookup API varies across SDK versions, so
            // the probe keeps to the core count; the Buses UI labels
            // channels by index the same way.
            out.push(ChannelInfo {
                index,
                name: format!("Kvaser 通道 {index}"),
            });
        }
        Ok(out)
    }
}

/// 手动真机验证（默认跳过）：
///
/// ```text
/// cargo test kvaser_live -- --ignored --nocapture
/// ```
///
/// 依次打开每个通道 BusOn，静默收 300 ms——验证驱动加载、通道打开、
/// BusOn 与非阻塞 read 全链可用，不往线上写任何帧。
/// 手动真机验证（默认跳过）：
///
/// ```text
/// cargo test kvaser_live -- --ignored --nocapture
/// ```
///
/// 依次尝试打开每个通道 BusOn，静默收 300 ms——验证驱动加载、通道
/// 打开、BusOn 与非阻塞 read 全链可用，不往线上写任何帧。单个通道
/// 打不开只记录并继续。
#[cfg(test)]
mod live {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    #[ignore = "需要本机 Kvaser 驱动：cargo test kvaser_live -- --ignored --nocapture"]
    fn kvaser_live_open_and_read() {
        let channels = enumerate().expect("驱动可用");
        println!("{} channel(s)", channels.len());
        let mut opened = 0usize;
        for c in &channels {
            match KvaserChannel::open(c.index, 500) {
                Ok(mut ch) => {
                    opened += 1;
                    let t0 = Instant::now();
                    let mut seen = 0usize;
                    while t0.elapsed() < Duration::from_millis(300) {
                        if ch.try_read().is_some() {
                            seen += 1;
                        }
                    }
                    println!("ch{}: ok, {} frame(s) in 300 ms", c.index, seen);
                }
                Err(e) => println!("ch{}: {e}", c.index),
            }
        }
        assert!(
            opened > 0,
            "没有一个通道能打开——status -3 (NOTFOUND) 表示通道已配置但适配器未插上/未上电"
        );
    }
}
