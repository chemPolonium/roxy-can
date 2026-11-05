//! Live Ethernet through Npcap's `wpcap.dll`. The SDK is not required: the
//! handful of libpcap entry points are loaded at runtime, so a machine without
//! Npcap still builds and simply reports "not installed".

use std::ffi::{CStr, CString, c_char, c_int, c_uchar, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

use crate::eth::EthFrame;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthInterface {
    pub device: String,
    pub description: String,
}

#[repr(C)]
struct PcapIf {
    next: *mut PcapIf,
    name: *const c_char,
    description: *const c_char,
    addresses: *mut c_void,
    flags: u32,
}

#[repr(C)]
struct PcapPkthdr {
    ts_sec: i32,
    ts_usec: i32,
    caplen: u32,
    len: u32,
}

type Pcap = *mut c_void;

struct Api {
    _lib: isize,
    findalldevs: unsafe extern "C" fn(*mut *mut PcapIf, *mut c_char) -> c_int,
    freealldevs: unsafe extern "C" fn(*mut PcapIf),
    open_live: unsafe extern "C" fn(*const c_char, c_int, c_int, c_int, *mut c_char) -> Pcap,
    close: unsafe extern "C" fn(Pcap),
    next_ex: unsafe extern "C" fn(Pcap, *mut *const PcapPkthdr, *mut *const c_uchar) -> c_int,
    sendpacket: unsafe extern "C" fn(Pcap, *const c_uchar, c_int) -> c_int,
    datalink: unsafe extern "C" fn(Pcap) -> c_int,
    geterr: unsafe extern "C" fn(Pcap) -> *const c_char,
}

// The function pointers are immutable after load; pcap handles are serialized
// by the owning mutex/thread.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

fn load() -> Result<Api, String> {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    let wide: Vec<u16> = "wpcap.dll\0".encode_utf16().collect();
    let lib = unsafe { LoadLibraryW(wide.as_ptr()) };
    if lib == 0 {
        return Err("Npcap wpcap.dll not found; install Npcap to use a live Ethernet port".into());
    }
    unsafe fn sym<T: Copy>(lib: isize, name: &[u8]) -> Result<T, String> {
        let proc = unsafe { GetProcAddress(lib, name.as_ptr()) };
        let Some(proc) = proc else {
            return Err(format!(
                "wpcap.dll is missing {}",
                String::from_utf8_lossy(&name[..name.len().saturating_sub(1)])
            ));
        };
        Ok(unsafe { std::mem::transmute_copy(&proc) })
    }
    Ok(Api {
        _lib: lib as isize,
        findalldevs: unsafe { sym(lib as isize, b"pcap_findalldevs\0") }?,
        freealldevs: unsafe { sym(lib as isize, b"pcap_freealldevs\0") }?,
        open_live: unsafe { sym(lib as isize, b"pcap_open_live\0") }?,
        close: unsafe { sym(lib as isize, b"pcap_close\0") }?,
        next_ex: unsafe { sym(lib as isize, b"pcap_next_ex\0") }?,
        sendpacket: unsafe { sym(lib as isize, b"pcap_sendpacket\0") }?,
        datalink: unsafe { sym(lib as isize, b"pcap_datalink\0") }?,
        geterr: unsafe { sym(lib as isize, b"pcap_geterr\0") }?,
    })
}

pub fn interfaces() -> Result<Vec<EthInterface>, String> {
    let api = load()?;
    let mut head: *mut PcapIf = std::ptr::null_mut();
    let mut err = [0 as c_char; 256];
    if unsafe { (api.findalldevs)(&mut head, err.as_mut_ptr()) } != 0 {
        let msg = unsafe { CStr::from_ptr(err.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        return Err(format!("pcap_findalldevs: {msg}"));
    }
    let mut out = Vec::new();
    let mut cur = head;
    while !cur.is_null() {
        let iface = unsafe { &*cur };
        let name = unsafe { CStr::from_ptr(iface.name) }
            .to_string_lossy()
            .into_owned();
        let description = if iface.description.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(iface.description) }
                .to_string_lossy()
                .into_owned()
        };
        // Loopback and Npcap's "pcap" pseudo-device are not useful for a bus.
        if !name.starts_with("rpcap") {
            out.push(EthInterface {
                device: name,
                description,
            });
        }
        cur = iface.next;
    }
    unsafe { (api.freealldevs)(head) };
    out.sort_by(|a, b| a.device.cmp(&b.device));
    Ok(out)
}

struct TxHandle(Pcap);
unsafe impl Send for TxHandle {}
unsafe impl Sync for TxHandle {}

pub struct EthPort {
    device: String,
    rx: Receiver<EthFrame>,
    tx: Arc<std::sync::Mutex<TxHandle>>,
    capture: isize,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
    api: Arc<Api>,
}

unsafe impl Send for EthPort {}

impl std::fmt::Debug for EthPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EthPort")
            .field("device", &self.device)
            .finish()
    }
}

impl EthPort {
    pub fn open(device: &str) -> Result<Self, String> {
        let api = Arc::new(load()?);
        let name = CString::new(device).map_err(|_| "invalid interface name".to_string())?;
        let mut err = [0 as c_char; 256];
        let handle = unsafe { (api.open_live)(name.as_ptr(), 65535, 1, 10, err.as_mut_ptr()) };
        if handle.is_null() {
            let msg = unsafe { CStr::from_ptr(err.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            return Err(format!("pcap_open_live: {msg}"));
        }
        if unsafe { (api.datalink)(handle) } != 1 {
            unsafe { (api.close)(handle) };
            return Err("interface is not Ethernet".into());
        }
        let mut tx_err = [0 as c_char; 256];
        let tx_handle =
            unsafe { (api.open_live)(name.as_ptr(), 65535, 0, 10, tx_err.as_mut_ptr()) };
        if tx_handle.is_null() {
            unsafe { (api.close)(handle) };
            let msg = unsafe { CStr::from_ptr(tx_err.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            return Err(format!("pcap_open_live tx: {msg}"));
        }
        let tx = Arc::new(std::sync::Mutex::new(TxHandle(tx_handle)));
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, rx) = channel();
        let thread_handle = handle as isize;
        let thread_api = Arc::clone(&api);
        let thread_stop = Arc::clone(&stop);
        let join = std::thread::Builder::new()
            .name("eth-rx".into())
            .spawn(move || {
                let handle = thread_handle as Pcap;
                while !thread_stop.load(Ordering::Relaxed) {
                    let mut header: *const PcapPkthdr = std::ptr::null();
                    let mut data: *const c_uchar = std::ptr::null();
                    let code = unsafe { (thread_api.next_ex)(handle, &mut header, &mut data) };
                    if code == 0 {
                        continue;
                    }
                    if code < 0 {
                        break;
                    }
                    let Some(h) = (unsafe { header.as_ref() }) else {
                        continue;
                    };
                    let len = h.caplen as usize;
                    if len < 14 {
                        continue;
                    }
                    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
                    let Some(frame) = frame_from_l2(bytes, h.ts_sec, h.ts_usec) else {
                        continue;
                    };
                    if sender.send(frame).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            device: device.to_string(),
            rx,
            tx,
            capture: handle as isize,
            stop,
            join: Some(join),
            api,
        })
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    pub fn try_read(&self) -> Option<EthFrame> {
        self.rx.try_recv().ok()
    }

    pub fn send(&self, frame: &EthFrame) -> Result<(), String> {
        let bytes = l2_bytes(frame);
        let handle = self.tx.lock().map_err(|_| "Ethernet port poisoned")?;
        let code = unsafe { (self.api.sendpacket)(handle.0, bytes.as_ptr(), bytes.len() as c_int) };
        if code == 0 {
            Ok(())
        } else {
            let msg = unsafe { CStr::from_ptr((self.api.geterr)(handle.0)) }
                .to_string_lossy()
                .into_owned();
            Err(format!("pcap_sendpacket: {msg}"))
        }
    }
}

impl Drop for EthPort {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
        if self.capture != 0 {
            unsafe { (self.api.close)(self.capture as Pcap) };
        }
        if let Ok(handle) = self.tx.lock()
            && !handle.0.is_null()
        {
            unsafe { (self.api.close)(handle.0) };
        }
    }
}

fn l2_bytes(frame: &EthFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(18 + frame.payload.len());
    out.extend_from_slice(&frame.dst);
    out.extend_from_slice(&frame.src);
    if let Some(vlan) = frame.vlan {
        out.extend_from_slice(&frame.vlan_tpid.unwrap_or(0x8100).to_be_bytes());
        out.extend_from_slice(&vlan.to_be_bytes());
    }
    out.extend_from_slice(&frame.ethertype.to_be_bytes());
    out.extend_from_slice(&frame.payload);
    out
}

fn frame_from_l2(bytes: &[u8], _sec: i32, _usec: i32) -> Option<EthFrame> {
    if bytes.len() < 14 {
        return None;
    }
    let mut dst = [0u8; 6];
    dst.copy_from_slice(&bytes[0..6]);
    let mut src = [0u8; 6];
    src.copy_from_slice(&bytes[6..12]);
    let mut at = 12usize;
    let mut vlan_tpid = None;
    let mut vlan = None;
    let mut ethertype = u16::from_be_bytes([bytes[at], bytes[at + 1]]);
    at += 2;
    while matches!(ethertype, 0x8100 | 0x88a8 | 0x9100) && bytes.len() >= at + 4 {
        vlan_tpid = Some(ethertype);
        vlan = Some(u16::from_be_bytes([bytes[at], bytes[at + 1]]));
        ethertype = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]);
        at += 4;
    }
    Some(EthFrame {
        t_us: 0,
        bus: 0,
        dir: crate::can::frame::Direction::Rx,
        dst,
        src,
        vlan,
        vlan_tpid,
        ethertype,
        payload: Arc::from(&bytes[at..]),
        seq: 0,
        truncated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npcap_is_either_usable_or_reports_a_clear_error() {
        match interfaces() {
            Ok(list) => {
                assert!(list.iter().all(|iface| !iface.device.is_empty()));
            }
            Err(error) => assert!(!error.is_empty()),
        }
    }
}
