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

// canlib32.h 的消息标志位值（canstat.h，canMSG_MASK = 0x00ff）。
const CAN_MSG_RTR: u32 = 0x0001;
const CAN_MSG_STD: u32 = 0x0002;
const CAN_MSG_EXT: u32 = 0x0004;
const CAN_MSG_ERROR_FRAME: u32 = 0x0020;
// CAN FD 标志住在高位字节（canFDMSG_MASK = 0xff0000）——低位字节的
// 0x0080 是 canMSG_TXRQ，与 FD 无关。TX 用 canWrite + FDF 标志
//（与官方迁移指南、python-can 一致），BRS 可选。
const CAN_FDMSG_FDF: u32 = 0x0001_0000;
const CAN_FDMSG_BRS: u32 = 0x0002_0000;
const CAN_FDMSG_ESI: u32 = 0x0004_0000;
/// canOPEN_NO_INIT_ACCESS: open for receiving only. Succeeds even when
/// another program holds the channel's init access (e.g. CAN King).
const CAN_OPEN_NO_INIT_ACCESS: i32 = 0x0020;
/// canOPEN_CAN_FD: open the channel with FD capability (needed before
/// FD data-phase params can be set / FD frames can leave).
const CAN_OPEN_CAN_FD: i32 = 0x0400;

/// Kvaser preset bitrate codes (negative = table entry; the tseg
/// arguments are ignored). Only exact matches are accepted: silently
/// wiring at the wrong speed would be far worse than refusing.
fn bitrate_code(kbps: u32) -> Option<i32> {
    Some(match kbps {
        1000 => -1,
        500 => -4,
        250 => -8,
        125 => -9,
        100 => -10,
        83 => -11,
        62 => -12,
        50 => -13,
        10 => -15,
        _ => return None,
    })
}

/// FD data-phase presets (canFD_BITRATE_*): bitrate + 80%/60% sample
/// point pairs, same exact-match policy as the arbitration table.
fn fd_bitrate_code(kbps: u32) -> Option<i32> {
    Some(match kbps {
        500 => -1000,  // canFD_BITRATE_500K_80P
        1000 => -1001, // canFD_BITRATE_1M_80P
        2000 => -1002, // canFD_BITRATE_2M_80P
        4000 => -1003, // canFD_BITRATE_4M_80P
        8000 => -1004, // canFD_BITRATE_8M_60P
        _ => return None,
    })
}

type CanInitializeLibrary = unsafe extern "system" fn() -> CanStatus;
type CanGetNumberOfChannels = unsafe extern "system" fn(*mut i32) -> CanStatus;
type CanOpenChannel = unsafe extern "system" fn(i32, i32) -> CanHandle;
type CanSetBusParams =
    unsafe extern "system" fn(i32, i32, u8, u8, u8, u8, u32) -> CanStatus;
type CanSetBusParamsFd =
    unsafe extern "system" fn(i32, i32, u8, u8, u8) -> CanStatus;
type CanGetChannelData =
    unsafe extern "system" fn(i32, i32, *mut c_void, *mut usize) -> CanStatus;
type CanBusOn = unsafe extern "system" fn(i32) -> CanStatus;
type CanBusOff = unsafe extern "system" fn(i32) -> CanStatus;
type CanWrite = unsafe extern "system" fn(i32, u32, *const u8, u32, u32) -> CanStatus;
type CanRead =
    unsafe extern "system" fn(i32, *mut u32, *mut u8, *mut u32, *mut u32, *mut u32) -> CanStatus;
type CanClose = unsafe extern "system" fn(i32) -> CanStatus;
type CanGetVersion = unsafe extern "system" fn() -> u32;

/// Every canlib entry the tool needs. `None` members mean the DLL or the
/// export was missing -- the wrapper reports "driver not available".
#[derive(Debug)]
struct Canlib {
    initialize_library: CanInitializeLibrary,
    get_number_of_channels: CanGetNumberOfChannels,
    /// Serves the ignored live bring-up test.
    #[allow(dead_code)]
    get_channel_data: CanGetChannelData,
    open_channel: CanOpenChannel,
    set_bus_params: CanSetBusParams,
    /// FD data-phase bitrate; absent on very old drivers.
    set_bus_params_fd: Option<CanSetBusParamsFd>,
    bus_on: CanBusOn,
    bus_off: CanBusOff,
    write: CanWrite,
    read: CanRead,
    close: CanClose,
    /// Serves the ignored live bring-up test.
    #[allow(dead_code)]
    get_version: CanGetVersion,
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
        macro_rules! want {
            ($name:expr, $ty:ty) => {
                // Optional export: missing on old drivers, degrades the
                // related feature instead of the whole binding.
                unsafe { std::mem::transmute::<*mut c_void, Option<$ty>>(sym($name)) }
            };
        }
        Some(Canlib {
            initialize_library: need!(b"canInitializeLibrary", CanInitializeLibrary),
            get_number_of_channels: need!(b"canGetNumberOfChannels", CanGetNumberOfChannels),
            get_channel_data: need!(b"canGetChannelData", CanGetChannelData),
            open_channel: need!(b"canOpenChannel", CanOpenChannel),
            set_bus_params: need!(b"canSetBusParams", CanSetBusParams),
            set_bus_params_fd: want!(b"canSetBusParamsFd", CanSetBusParamsFd),
            bus_on: need!(b"canBusOn", CanBusOn),
            bus_off: need!(b"canBusOff", CanBusOff),
            write: need!(b"canWrite", CanWrite),
            read: need!(b"canRead", CanRead),
            close: need!(b"canClose", CanClose),
            get_version: need!(b"canGetVersion", CanGetVersion),
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
    /// FD data-phase params were applied at open time; FD frames can
    /// leave this channel.
    pub fd: bool,
}

impl KvaserChannel {
    /// Opens a channel and puts the bus on at the given bitrate.
    ///
    /// `init_access = false` opens receive-only (canOPEN_NO_INIT_ACCESS):
    /// it succeeds even when another program holds the channel — the
    /// monitoring path. `init_access = true` allows writing frames but
    /// fails while the channel is held elsewhere.
    ///
    /// `fd_data_kbps` opts the channel into CAN FD: the channel opens
    /// with canOPEN_CAN_FD and the FD data-phase preset is applied. When
    /// no preset matches (or the channel/hardware refuses FD), the
    /// channel degrades to classic and `fd` comes back false — the
    /// status line reports it, FD frames then fail on write instead of
    /// leaving at a silently wrong speed.
    pub fn open(
        index: i32,
        kbps: u32,
        fd_data_kbps: Option<u32>,
        init_access: bool,
    ) -> Result<KvaserChannel, String> {
        let lib = Canlib::lib().ok_or("Kvaser 驱动不可用（canlib32.dll 未找到）")?;
        let fd_code = fd_data_kbps.and_then(fd_bitrate_code);
        unsafe {
            (lib.initialize_library)();
            let mut flags = if init_access { 0 } else { CAN_OPEN_NO_INIT_ACCESS };
            if fd_code.is_some() {
                flags |= CAN_OPEN_CAN_FD;
            }
            let mut handle = (lib.open_channel)(index, flags);
            let mut fd = fd_code.is_some();
            if handle < 0 && fd_code.is_some() {
                // Adapter or driver without FD support: retry classic.
                fd = false;
                handle = (lib.open_channel)(index, flags & !CAN_OPEN_CAN_FD);
            }
            if handle < 0 {
                return Err(format!("打开 Kvaser 通道 {index} 失败（status {handle}）"));
            }
            let Some(code) = bitrate_code(kbps) else {
                (lib.close)(handle);
                return Err(format!(
                    "不支持的波特率 {kbps} kbit/s（支持 10/50/62/83/100/125/250/500/1000）"
                ));
            };
            let status = (lib.set_bus_params)(handle, code, 0, 0, 0, 0, 0);
            if status != CAN_OK {
                (lib.close)(handle);
                return Err(format!("设置波特率失败（status {status}）"));
            }
            if let (true, Some(code)) = (fd, fd_code) {
                match lib.set_bus_params_fd {
                    Some(set_fd) => {
                        let status = set_fd(handle, code, 0, 0, 0);
                        if status != CAN_OK {
                            (lib.close)(handle);
                            return Err(format!("设置 FD 数据段波特率失败（status {status}）"));
                        }
                    }
                    None => {
                        // Driver predates FD: fall back to classic.
                        (lib.close)(handle);
                        return Self::open(index, kbps, None, init_access);
                    }
                }
            }
            let status = (lib.bus_on)(handle);
            if status != CAN_OK {
                (lib.close)(handle);
                return Err(format!("BusOn 失败（status {status}）"));
            }
            Ok(KvaserChannel { handle, fd })
        }
    }

    /// Writes one frame out. Ids over 0x7FF go extended; RTR frames keep
    /// their flag and carry no payload. FD frames (payload up to 64
    /// bytes) go out with the canFDMSG_FDF marker, BRS following the
    /// frame's flag — the same canWrite call the official migration
    /// guide uses, with the FD bits in the high flag byte.
    pub fn write_frame(&self, f: &CanFrame) -> Result<(), String> {
        let lib = Canlib::lib().ok_or("Kvaser 驱动不可用")?;
        // RTR carries no payload but its `len` is the requested byte
        // count: the DLC goes out as the request, not the empty payload.
        let len = if f.is_remote() {
            f.len as u32
        } else {
            f.payload().len().min(MAX_CAN_FD_LEN) as u32
        };
        let mut flag = if f.extended {
            CAN_MSG_EXT
        } else if f.is_remote() {
            CAN_MSG_RTR
        } else {
            CAN_MSG_STD
        };
        if f.is_fd() {
            flag |= CAN_FDMSG_FDF;
            if f.flags.contains(FrameFlags::BRS) {
                flag |= CAN_FDMSG_BRS;
            }
            if f.flags.contains(FrameFlags::ESI) {
                flag |= CAN_FDMSG_ESI;
            }
        }
        // canWrite copies `len` bytes from the buffer even for RTR
        // frames (whose payload slice is empty, its pointer dangling):
        // point it at the full fixed array, whose first `len` bytes are
        // in-bounds by construction.
        let status = unsafe { (lib.write)(self.handle, f.id, f.data.as_ptr(), len, flag) };
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
        let is_error = flag & CAN_MSG_ERROR_FRAME != 0;
        let is_fd = flag & CAN_FDMSG_FDF != 0;
        let len = dlc.min(MAX_CAN_FD_LEN as u32) as u8;
        let mut flags = FrameFlags::NONE;
        if is_fd {
            flags = flags.union(FrameFlags::FD);
        }
        if flag & CAN_FDMSG_BRS != 0 {
            flags = flags.union(FrameFlags::BRS);
        }
        if flag & CAN_FDMSG_ESI != 0 {
            flags = flags.union(FrameFlags::ESI);
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

#[cfg(test)]
mod tables {
    use super::*;

    #[test]
    fn bitrate_tables_accept_only_their_declared_preset_values() {
        // Arbitration presets: every documented value, then a near miss.
        for (kbps, code) in [
            (1000, -1),
            (500, -4),
            (250, -8),
            (125, -9),
            (100, -10),
            (83, -11),
            (62, -12),
            (50, -13),
            (10, -15),
        ] {
            assert_eq!(bitrate_code(kbps), Some(code), "{kbps} kbit/s");
        }
        for bad in [0, 1, 95, 200, 333, 999, 2000] {
            assert_eq!(bitrate_code(bad), None, "{bad} kbit/s must refuse");
        }

        // FD data-phase presets: same exact-match policy.
        for (kbps, code) in [
            (500, -1000),
            (1000, -1001),
            (2000, -1002),
            (4000, -1003),
            (8000, -1004),
        ] {
            assert_eq!(fd_bitrate_code(kbps), Some(code), "FD {kbps} kbit/s");
        }
        for bad in [0, 250, 750, 400, 5000] {
            assert_eq!(fd_bitrate_code(bad), None, "FD {bad} must refuse");
        }
    }

    /// The flag bits are load-bearing for both directions: TX sets them,
    /// RX decodes them. A wrong constant would mislabel every FD frame
    /// (0x0080, once guessed, is actually canMSG_TXRQ). These mirror
    /// canstat.h; keep them in sync with it.
    #[test]
    fn fd_flag_bits_match_canstat_h() {
        assert_eq!(CAN_FDMSG_FDF, 0x0001_0000);
        assert_eq!(CAN_FDMSG_BRS, 0x0002_0000);
        assert_eq!(CAN_FDMSG_ESI, 0x0004_0000);
        assert_eq!(CAN_MSG_RTR, 0x0001);
        assert_eq!(CAN_MSG_STD, 0x0002);
        assert_eq!(CAN_MSG_EXT, 0x0004);
        assert_eq!(CAN_MSG_ERROR_FRAME, 0x0020);
        assert_eq!(CAN_OPEN_NO_INIT_ACCESS, 0x0020);
        assert_eq!(CAN_OPEN_CAN_FD, 0x0400);
    }
}

/// 手动真机验证（默认跳过）：
///
/// ```text
/// cargo test kvaser_live -- --ignored --nocapture
/// ```
///
/// 虚拟通道回环诊断：ch0 只收收听，ch1 只收发送——若虚拟网络在通道
/// 间路由帧，ch0 应收到 ch1 写的帧（证明 NO_INIT 句柄可发车），并
/// 顺带验证经典帧与 FD 帧的标志位编解码。单个通道打不开只记录并继续。
#[cfg(test)]
mod live {
    use super::*;
    use std::time::{Duration, Instant};

    /// 诊断矩阵：逐通道试 init access / FD 的开关组合，找出虚拟通道
    /// 拒绝收发挂接的确切条件。只开即关，不往线上写任何帧。
    #[test]
    #[ignore = "需要本机 Kvaser 驱动：cargo test kvaser_open_matrix -- --ignored --nocapture"]
    fn kvaser_open_matrix() {
        let channels = enumerate().expect("驱动可用");
        println!("canlib version raw: {:#x}", unsafe {
            (Canlib::lib().expect("lib").get_version)()
        });
        println!("{} channel(s)", channels.len());
        let lib = Canlib::lib().expect("lib");
        for c in &channels {
            // 裸标志矩阵：ACCEPT_VIRTUAL(0x8000) 与 init access 的组合
            // ——python-can 开虚拟通道默认带 ACCEPT_VIRTUAL + init。
            for (label, flags) in [
                ("0x8000 AV", 0x8000),
                ("0x8020 AV|NOINIT", 0x8020),
                ("0x8400 AV|FD", 0x8400),
                ("0x0000 init", 0x0000),
            ] {
                let h = unsafe { (lib.open_channel)(c.index, flags) };
                println!("ch{} {label}: handle {h}", c.index);
                if h >= 0 {
                    unsafe {
                        (lib.bus_off)(h);
                        (lib.close)(h);
                    }
                }
            }
        }
        for c in &channels {
            for (label, fd, init) in [
                ("init+FD", Some(2000u32), true),
                ("init classic", None, true),
                ("rx+FD", Some(2000), false),
                ("rx classic", None, false),
            ] {
                match KvaserChannel::open(c.index, 500, fd, init) {
                    Ok(h) => println!("ch{} {label}: OK (fd={})", c.index, h.fd),
                    Err(e) => println!("ch{} {label}: FAIL ({e})", c.index),
                }
            }
        }
    }

    /// 虚拟通道回环：ch1 发、ch0 收，验证经典/FD/RTR 的标志位编解码
    /// 与 NO_INIT 句柄的可发车性。
    #[test]
    #[ignore = "需要本机 Kvaser 驱动：cargo test kvaser_live -- --ignored --nocapture"]
    fn kvaser_live_open_and_read() {
        let channels = enumerate().expect("驱动可用");
        println!("canlib version raw: {:#x}", unsafe {
            (Canlib::lib().expect("lib").get_version)()
        });
        println!("{} channel(s)", channels.len());

        // 回环诊断：ch0 只收收听，ch1 只收发送——若虚拟网络在通道间
        // 路由帧，ch0 应收到 ch1 写的帧（证明 NO_INIT 句柄可发车）。
        // 走封装的 open（含 FD 预设路径），而不是裸标志。
        let (mut rx, tx) = {
            let rx = KvaserChannel::open(0, 500, Some(2000), false);
            let tx = KvaserChannel::open(1, 500, Some(2000), false);
            println!("rx fd={:?}", rx.as_ref().map(|c| c.fd));
            println!("tx fd={:?}", tx.as_ref().map(|c| c.fd));
            match (rx, tx) {
                (Ok(rx), Ok(tx)) => (rx, tx),
                (e, _) => panic!("open failed: {e:?}"),
            }
        };
        let classic = CanFrame {
            t_us: 0,
            channel: 0,
            id: 0x555,
            extended: false,
            len: 3,
            data: {
                let mut d = [0u8; MAX_CAN_FD_LEN];
                d[0] = 0x42;
                d
            },
            dir: Direction::Tx,
            flags: FrameFlags::NONE,
        };
        let mut fd_frame = classic;
        fd_frame.id = 0x556;
        fd_frame.len = 12;
        fd_frame.flags = FrameFlags::FD.union(FrameFlags::BRS);
        let mut rtr = classic;
        rtr.id = 0x557;
        rtr.len = 8; // the requested byte count goes out as the DLC
        rtr.flags = FrameFlags::RTR;
        let mut sent_c = 0usize;
        let mut sent_f = 0usize;
        let mut sent_r = 0usize;
        let mut seen_c = 0usize;
        let mut seen_f = 0usize;
        let mut rtr_seen = 0usize;
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_millis(2000) {
            if sent_c < 5 && tx.write_frame(&classic).is_ok() {
                sent_c += 1;
            }
            if sent_f < 5 && tx.write_frame(&fd_frame).is_ok() {
                sent_f += 1;
            }
            if sent_r < 5 && tx.write_frame(&rtr).is_ok() {
                sent_r += 1;
            }
            while let Some(f) = rx.try_read() {
                if f.flags.contains(FrameFlags::FD) {
                    seen_f += 1;
                } else {
                    seen_c += 1;
                }
                if f.flags.contains(FrameFlags::RTR) {
                    rtr_seen += 1;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        println!(
            "sent classic {sent_c}, fd {sent_f}, rtr {sent_r}; received classic {seen_c}, fd {seen_f} (rtr {rtr_seen})"
        );
        drop(rx);
        drop(tx);
    }
}