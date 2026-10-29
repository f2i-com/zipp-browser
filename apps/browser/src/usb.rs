//! WebUSB (`navigator.usb`, `webusb_shim.js`) on Linux's usbfs. A page talks
//! to USB devices itself -- control, bulk and interrupt transfers -- which is
//! enough to write a driver in JavaScript for a device the kernel has none
//! for: a Bluetooth dongle, a serial adapter, a microcontroller.
//!
//! Devices are listed from sysfs, with their descriptors read from
//! /dev/bus/usb/BBB/DDD, which is also what a page opens. Control transfers
//! are synchronous. Bulk and interrupt transfers are submitted as URBs and
//! collected by a thread per open device, which wakes the window: the event
//! bridge hands each completion to the page (`__zippUsbComplete`).
//!
//! Who may: `ZIPP_WEBUSB=on` lets every page, `off` none; by default local
//! pages (`file://`, the app a zipp-linux image was built with) get the
//! devices and remote ones do not. There is no chooser: `requestDevice`
//! grants the first device its filters match. An interface the kernel drives
//! is taken from its driver when claimed -- except a HID device's or a
//! drive's, which are the keyboard, the mouse and the disks.

use std::collections::HashMap;
use std::ffi::c_void;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value as Json, json};

/// `_IOC(dir, 'U', nr, size)`, the same on x86_64 and arm64.
const fn ioc(dir: u64, nr: u64, size: usize) -> u64 {
    (dir << 30) | ((size as u64) << 16) | ((b'U' as u64) << 8) | nr
}
const READ: u64 = 2;
const WRITE: u64 = 1;
const USBDEVFS_CONTROL: u64 = ioc(READ | WRITE, 0, size_of::<CtrlTransfer>());
const USBDEVFS_SETINTERFACE: u64 = ioc(READ, 4, 8);
const USBDEVFS_SETCONFIGURATION: u64 = ioc(READ, 5, 4);
const USBDEVFS_SUBMITURB: u64 = ioc(READ, 10, size_of::<Urb>());
const USBDEVFS_REAPURBNDELAY: u64 = ioc(WRITE, 13, size_of::<*mut c_void>());
const USBDEVFS_CLAIMINTERFACE: u64 = ioc(READ, 15, 4);
const USBDEVFS_RELEASEINTERFACE: u64 = ioc(READ, 16, 4);
const USBDEVFS_RESET: u64 = ioc(0, 20, 0);
const USBDEVFS_CLEAR_HALT: u64 = ioc(READ, 21, 4);
const USBDEVFS_DISCONNECT_CLAIM: u64 = ioc(READ, 27, size_of::<DisconnectClaim>());

const URB_TYPE_INTERRUPT: u8 = 1;
const URB_TYPE_BULK: u8 = 3;

/// Interface classes never taken from the kernel: HID and mass storage.
const SYSTEM_CLASSES: [u8; 2] = [0x03, 0x08];

#[repr(C)]
struct CtrlTransfer {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
    timeout: u32,
    data: *mut c_void,
}

#[repr(C)]
struct Urb {
    kind: u8,
    endpoint: u8,
    status: i32,
    flags: u32,
    buffer: *mut c_void,
    buffer_length: i32,
    actual_length: i32,
    start_frame: i32,
    number_of_packets: i32,
    error_count: i32,
    signr: u32,
    usercontext: *mut c_void,
}

#[repr(C)]
struct DisconnectClaim {
    interface: u32,
    flags: u32,
    driver: [u8; 256],
}

fn ioctl<T>(fd: RawFd, request: u64, arg: *mut T) -> std::io::Result<i32> {
    let rc = unsafe { libc::ioctl(fd, request as _, arg) };
    if rc < 0 { Err(std::io::Error::last_os_error()) } else { Ok(rc) }
}

/// A transfer the kernel has, with the memory it reads and writes.
struct Inflight {
    id: u64,
    dir_in: bool,
    urb: Box<Urb>,
    buffer: Vec<u8>,
}

// SAFETY: the URB's pointers are into `buffer`, which travels with it.
unsafe impl Send for Inflight {}

type InflightMap = Arc<Mutex<HashMap<usize, Inflight>>>;

struct Device {
    file: File,
    sysfs: PathBuf,
    inflight: InflightMap,
    stop: OwnedFd,
    reaper: Option<JoinHandle<()>>,
}

impl Drop for Device {
    fn drop(&mut self) {
        let one: u64 = 1;
        unsafe { libc::write(self.stop.as_raw_fd(), (&one as *const u64).cast(), 8) };
        if let Some(reaper) = self.reaper.take() {
            let _ = reaper.join();
        }
        // Closing the file cancels whatever is still in flight; only then
        // may the memory it points into go.
    }
}

static DEVICES: Mutex<Option<HashMap<String, Device>>> = Mutex::new(None);
static COMPLETIONS: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());

/// Transfers finished since last asked, as `(id, result JSON)`.
pub(crate) fn take_completions() -> Vec<(u64, String)> {
    std::mem::take(&mut *COMPLETIONS.lock().unwrap_or_else(|e| e.into_inner()))
}

fn allowed() -> bool {
    match std::env::var("ZIPP_WEBUSB").as_deref() {
        Ok("on") => true,
        Ok("off") => false,
        _ => crate::CURRENT_URL.lock().map(|u| u.starts_with("file://")).unwrap_or(false),
    }
}

/// `__zippHostCall("usb", op, ...)`.
pub(crate) fn host_call(args: &[String]) -> Result<String, String> {
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
    let num = |i: usize| arg(i).parse::<u64>().map_err(|_| format!("Error: usb: argument {i} is not a number"));
    if arg(0) == "list" {
        return Ok(if allowed() { list().to_string() } else { "[]".into() });
    }
    if !allowed() {
        return Err("SecurityError: USB is not available to this page".into());
    }
    let id = arg(1);
    let mut devices = DEVICES.lock().unwrap_or_else(|e| e.into_inner());
    let devices = devices.get_or_insert_with(HashMap::new);
    match arg(0) {
        "open" => {
            if !devices.contains_key(id) {
                devices.insert(id.to_string(), open(id)?);
            }
            Ok(String::new())
        }
        "close" => {
            devices.remove(id);
            Ok(String::new())
        }
        op => {
            let device = devices.get(id).ok_or("InvalidStateError: the device is not open")?;
            let fd = device.file.as_raw_fd();
            match op {
                "select_configuration" => {
                    let mut value = num(2)? as u32;
                    ioctl(fd, USBDEVFS_SETCONFIGURATION, &mut value).map_err(|e| format!("NetworkError: {e}"))?;
                    Ok(String::new())
                }
                "claim" => claim(device, num(2)? as u32).map(|_| String::new()),
                "release" => {
                    let mut iface = num(2)? as u32;
                    ioctl(fd, USBDEVFS_RELEASEINTERFACE, &mut iface).map_err(|e| format!("NetworkError: {e}"))?;
                    Ok(String::new())
                }
                "alternate" => {
                    let mut setting = [num(2)? as u32, num(3)? as u32];
                    ioctl(fd, USBDEVFS_SETINTERFACE, setting.as_mut_ptr()).map_err(|e| format!("NetworkError: {e}"))?;
                    Ok(String::new())
                }
                "control" => control(fd, args),
                "transfer" => {
                    // transfer, device, id, endpoint address, "bulk"|"interrupt", length | base64
                    let endpoint = num(3)? as u8;
                    let kind = if arg(4) == "interrupt" { URB_TYPE_INTERRUPT } else { URB_TYPE_BULK };
                    let buffer = if endpoint & 0x80 != 0 {
                        vec![0u8; num(5)? as usize]
                    } else {
                        B64.decode(arg(5)).map_err(|e| format!("TypeError: {e}"))?
                    };
                    submit(device, num(2)?, kind, endpoint, buffer).map(|_| String::new())
                }
                "clear_halt" => {
                    let mut endpoint = num(2)? as u32;
                    ioctl(fd, USBDEVFS_CLEAR_HALT, &mut endpoint).map_err(|e| format!("NetworkError: {e}"))?;
                    Ok(String::new())
                }
                "reset" => {
                    ioctl(fd, USBDEVFS_RESET, std::ptr::null_mut::<c_void>()).map_err(|e| format!("NetworkError: {e}"))?;
                    Ok(String::new())
                }
                _ => Err(format!("Error: usb: no such operation {op}")),
            }
        }
    }
}

fn open(id: &str) -> Result<Device, String> {
    let (sysfs, node) = find(id).ok_or("NotFoundError: the device is gone")?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&node)
        .map_err(|e| format!("SecurityError: cannot open {}: {e}", node.display()))?;
    let stop = unsafe {
        let fd = libc::eventfd(0, libc::EFD_CLOEXEC);
        if fd < 0 {
            return Err(format!("Error: {}", std::io::Error::last_os_error()));
        }
        OwnedFd::from_raw_fd(fd)
    };
    let inflight: InflightMap = Arc::default();
    let reaper = {
        let (fd, stop, inflight) = (file.as_raw_fd(), stop.as_raw_fd(), inflight.clone());
        std::thread::Builder::new()
            .name("usb".into())
            .spawn(move || reap(fd, stop, inflight))
            .map_err(|e| format!("Error: {e}"))?
    };
    Ok(Device { file, sysfs, inflight, stop, reaper: Some(reaper) })
}

/// Claim an interface, taking it from the kernel's driver when it has one
/// and it is not one the system needs.
fn claim(device: &Device, iface: u32) -> Result<(), String> {
    let fd = device.file.as_raw_fd();
    let mut number = iface;
    match ioctl(fd, USBDEVFS_CLAIMINTERFACE, &mut number) {
        Ok(_) => return Ok(()),
        Err(e) if e.raw_os_error() != Some(libc::EBUSY) => return Err(format!("NetworkError: {e}")),
        Err(_) => {}
    }
    let class = interface_class(&device.sysfs, iface);
    if class.is_some_and(|c| SYSTEM_CLASSES.contains(&c)) {
        return Err("SecurityError: the interface is the system's (HID or storage)".into());
    }
    let mut request = DisconnectClaim { interface: iface, flags: 0, driver: [0; 256] };
    ioctl(fd, USBDEVFS_DISCONNECT_CLAIM, &mut request).map_err(|e| format!("NetworkError: {e}"))?;
    Ok(())
}

/// An interface's class, from its sysfs entry (`<device>:<config>.<interface>`).
fn interface_class(sysfs: &Path, iface: u32) -> Option<u8> {
    let name = sysfs.file_name()?.to_str()?.to_string();
    let config = std::fs::read_to_string(sysfs.join("bConfigurationValue")).ok()?;
    let path = sysfs.join(format!("{name}:{}.{iface}", config.trim())).join("bInterfaceClass");
    u8::from_str_radix(std::fs::read_to_string(path).ok()?.trim(), 16).ok()
}

/// control, device, "in"|"out", type, recipient, request, value, index, length | base64
fn control(fd: RawFd, args: &[String]) -> Result<String, String> {
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
    let num = |i: usize| arg(i).parse::<u16>().map_err(|_| format!("TypeError: control argument {i}"));
    let dir_in = arg(2) == "in";
    let kind = match arg(3) {
        "standard" => 0,
        "class" => 1,
        "vendor" => 2,
        other => return Err(format!("TypeError: request type {other}")),
    };
    let recipient = match arg(4) {
        "device" => 0,
        "interface" => 1,
        "endpoint" => 2,
        "other" => 3,
        other => return Err(format!("TypeError: recipient {other}")),
    };
    let mut data = if dir_in { vec![0u8; num(8)? as usize] } else { B64.decode(arg(8)).map_err(|e| format!("TypeError: {e}"))? };
    let mut transfer = CtrlTransfer {
        request_type: (u8::from(dir_in) << 7) | (kind << 5) | recipient,
        request: num(5)? as u8,
        value: num(6)?,
        index: num(7)?,
        length: data.len() as u16,
        timeout: 5000,
        data: data.as_mut_ptr().cast(),
    };
    let result = match ioctl(fd, USBDEVFS_CONTROL, &mut transfer) {
        Ok(n) if dir_in => json!({ "status": "ok", "data": B64.encode(&data[..n as usize]) }),
        Ok(n) => json!({ "status": "ok", "bytesWritten": n }),
        Err(e) if e.raw_os_error() == Some(libc::EPIPE) => json!({ "status": "stall", "data": "", "bytesWritten": 0 }),
        Err(e) => return Err(format!("NetworkError: {e}")),
    };
    Ok(result.to_string())
}

fn submit(device: &Device, id: u64, kind: u8, endpoint: u8, mut buffer: Vec<u8>) -> Result<(), String> {
    let mut urb = Box::new(Urb {
        kind,
        endpoint,
        status: 0,
        flags: 0,
        buffer: buffer.as_mut_ptr().cast(),
        buffer_length: buffer.len() as i32,
        actual_length: 0,
        start_frame: 0,
        number_of_packets: 0,
        error_count: 0,
        signr: 0,
        usercontext: std::ptr::null_mut(),
    });
    let key = &mut *urb as *mut Urb as usize;
    // Recorded before the kernel has it, so the reaper always finds it.
    let mut inflight = device.inflight.lock().unwrap_or_else(|e| e.into_inner());
    let urb_ptr: *mut Urb = &mut *urb;
    inflight.insert(key, Inflight { id, dir_in: endpoint & 0x80 != 0, urb, buffer });
    if let Err(e) = ioctl(device.file.as_raw_fd(), USBDEVFS_SUBMITURB, urb_ptr) {
        inflight.remove(&key);
        return Err(format!("NetworkError: {e}"));
    }
    Ok(())
}

/// Collect a device's finished transfers until it is closed or unplugged.
fn reap(fd: RawFd, stop: RawFd, inflight: InflightMap) {
    loop {
        let mut fds = [
            libc::pollfd { fd, events: libc::POLLOUT, revents: 0 },
            libc::pollfd { fd: stop, events: libc::POLLIN, revents: 0 },
        ];
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) } < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return;
        }
        if fds[1].revents != 0 {
            return;
        }
        let mut finished = Vec::new();
        loop {
            let mut urb: *mut Urb = std::ptr::null_mut();
            if ioctl(fd, USBDEVFS_REAPURBNDELAY, &mut urb).is_err() {
                break;
            }
            if let Some(done) = inflight.lock().unwrap_or_else(|e| e.into_inner()).remove(&(urb as usize)) {
                finished.push(done);
            }
        }
        let gone = fds[0].revents & (libc::POLLERR | libc::POLLHUP) != 0;
        if gone {
            // Unplugged: everything still in flight fails.
            finished.extend(inflight.lock().unwrap_or_else(|e| e.into_inner()).drain().map(|(_, t)| t));
        }
        if !finished.is_empty() {
            let mut completions = COMPLETIONS.lock().unwrap_or_else(|e| e.into_inner());
            for t in finished {
                completions.push((t.id, result(&t, gone).to_string()));
            }
            drop(completions);
            crate::worker::wake();
        }
        if gone {
            return;
        }
    }
}

fn result(t: &Inflight, gone: bool) -> Json {
    let status = match -t.urb.status {
        _ if gone && t.urb.status == 0 && t.urb.actual_length == 0 => return json!({ "error": "the device is gone" }),
        0 => "ok",
        libc::EPIPE => "stall",
        libc::EOVERFLOW => "babble",
        libc::ENOENT | libc::ECONNRESET => return json!({ "error": "the transfer was cancelled" }),
        libc::ENODEV | libc::ESHUTDOWN => return json!({ "error": "the device is gone" }),
        e => return json!({ "error": std::io::Error::from_raw_os_error(e).to_string() }),
    };
    let n = t.urb.actual_length.max(0) as usize;
    if t.dir_in {
        json!({ "status": status, "data": B64.encode(&t.buffer[..n.min(t.buffer.len())]) })
    } else {
        json!({ "status": status, "bytesWritten": n })
    }
}

/// The sysfs directory and device node of `bus-address`.
fn find(id: &str) -> Option<(PathBuf, PathBuf)> {
    let (bus, address) = id.split_once('-')?;
    let (bus, address): (u32, u32) = (bus.parse().ok()?, address.parse().ok()?);
    std::fs::read_dir("/sys/bus/usb/devices").ok()?.flatten().find_map(|e| {
        let path = e.path();
        let read = |f: &str| std::fs::read_to_string(path.join(f)).ok().and_then(|s| s.trim().parse::<u32>().ok());
        (read("busnum") == Some(bus) && read("devnum") == Some(address))
            .then(|| (path.clone(), PathBuf::from(format!("/dev/bus/usb/{bus:03}/{address:03}"))))
    })
}

/// Every USB device but hubs, as `navigator.usb` describes them.
fn list() -> Json {
    let Ok(dir) = std::fs::read_dir("/sys/bus/usb/devices") else { return json!([]) };
    let mut paths: Vec<PathBuf> = dir.flatten().map(|e| e.path()).filter(|p| p.join("busnum").exists()).collect();
    paths.sort();
    let devices: Vec<Json> = paths.iter().filter_map(|p| describe(p)).collect();
    Json::Array(devices)
}

fn describe(sysfs: &Path) -> Option<Json> {
    let text = |f: &str| std::fs::read_to_string(sysfs.join(f)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let bus: u32 = text("busnum")?.parse().ok()?;
    let address: u32 = text("devnum")?.parse().ok()?;
    let raw = std::fs::read(format!("/dev/bus/usb/{bus:03}/{address:03}")).ok()?;
    if raw.len() < 18 || raw[1] != 1 || raw[4] == 0x09 {
        return None; // not a device descriptor, or a hub
    }
    let word = |i: usize| u16::from_le_bytes([raw[i], raw[i + 1]]);
    let bcd = |v: u16| [v >> 8, (v >> 4) & 0xf, v & 0xf];
    let (usb, version) = (bcd(word(2)), bcd(word(12)));
    let active: Option<u8> = text("bConfigurationValue").and_then(|v| v.parse().ok());
    Some(json!({
        "id": format!("{bus}-{address}"),
        "usbVersionMajor": usb[0], "usbVersionMinor": usb[1], "usbVersionSubminor": usb[2],
        "deviceClass": raw[4], "deviceSubclass": raw[5], "deviceProtocol": raw[6],
        "vendorId": word(8), "productId": word(10),
        "deviceVersionMajor": version[0], "deviceVersionMinor": version[1], "deviceVersionSubminor": version[2],
        "manufacturerName": text("manufacturer"),
        "productName": text("product"),
        "serialNumber": text("serial"),
        "activeConfiguration": active,
        "configurations": configurations(&raw[raw[0] as usize..]),
    }))
}

/// The configuration descriptors that follow the device descriptor, with
/// their interfaces and endpoints.
fn configurations(mut raw: &[u8]) -> Json {
    let mut configs = Vec::new();
    while raw.len() >= 9 && raw[1] == 2 {
        let total = (u16::from_le_bytes([raw[2], raw[3]]) as usize).clamp(9, raw.len());
        let (config, rest) = raw.split_at(total);
        raw = rest;
        let mut interfaces: Vec<(u8, Vec<Json>)> = Vec::new();
        let mut i = config[0] as usize;
        while i + 2 <= config.len() && config[i] >= 2 {
            let d = &config[i..(i + config[i] as usize).min(config.len())];
            match d[1] {
                4 if d.len() >= 9 => {
                    let alternate = json!({
                        "alternateSetting": d[3],
                        "interfaceClass": d[5], "interfaceSubclass": d[6], "interfaceProtocol": d[7],
                        "interfaceName": null,
                        "endpoints": [],
                    });
                    match interfaces.iter_mut().find(|(n, _)| *n == d[2]) {
                        Some((_, alts)) => alts.push(alternate),
                        None => interfaces.push((d[2], vec![alternate])),
                    }
                }
                5 if d.len() >= 7 => {
                    if let Some(endpoints) = interfaces
                        .last_mut()
                        .and_then(|(_, alts)| alts.last_mut())
                        .and_then(|a| a["endpoints"].as_array_mut())
                    {
                        let kind = match d[3] & 3 {
                            0 => "control",
                            1 => "isochronous",
                            2 => "bulk",
                            _ => "interrupt",
                        };
                        endpoints.push(json!({
                            "endpointNumber": d[2] & 0x0f,
                            "direction": if d[2] & 0x80 != 0 { "in" } else { "out" },
                            "type": kind,
                            "packetSize": u16::from_le_bytes([d[4], d[5]]) & 0x7ff,
                        }));
                    }
                }
                _ => {}
            }
            i += config[i] as usize;
        }
        configs.push(json!({
            "configurationValue": config[5],
            "configurationName": null,
            "interfaces": interfaces.into_iter().map(|(n, alts)| json!({ "interfaceNumber": n, "alternates": alts })).collect::<Vec<_>>(),
        }));
    }
    Json::Array(configs)
}
