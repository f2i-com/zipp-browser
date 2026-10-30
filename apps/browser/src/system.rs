//! `navigator.zipp` (`zipp_shim.js`): the machine itself, for the apps that
//! are the machine's own -- files and drives, programs, the network, power.
//!
//! Only a `file:` page in `--kiosk` (zipp-linux, where pages on the device's
//! own disk are its apps) gets it, or any page when `ZIPP_SYSTEM=on`; on the
//! desktop a downloaded page opened from disk must not get the machine.
//!
//! Files cross in `__zippHostBytes`, in pieces the page asks for. The boot
//! drive (`/boot`, read-only while running) is made writable for each change
//! and read-only again after, under the lock zipp-init takes to write its log.
//! A program the page runs is waited for on a thread of its own; its result
//! comes back through the event bridge (`__zippSystemComplete`), as a USB
//! transfer's does.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use zipp_vm::embed::HostCtx;

/// Set by `--kiosk`.
pub(crate) static KIOSK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

static COMPLETIONS: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());
/// Which page programs belong to: one the last page started that ends under
/// the next must not settle that page's promise of the same number.
static PAGE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A new page: what the last one ran is no longer anyone's to hear about.
pub(crate) fn new_page() {
    PAGE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    COMPLETIONS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    #[cfg(target_os = "linux")]
    crate::pty::new_page();
}

/// Programs finished since last asked, as `(id, result JSON)`.
pub(crate) fn take_completions() -> Vec<(u64, String)> {
    std::mem::take(&mut *COMPLETIONS.lock().unwrap_or_else(|e| e.into_inner()))
}

fn allowed() -> bool {
    match std::env::var("ZIPP_SYSTEM").as_deref() {
        Ok("on") => true,
        Ok("off") => false,
        _ => {
            KIOSK.load(std::sync::atomic::Ordering::Relaxed)
                && crate::CURRENT_URL.lock().map(|u| u.starts_with("file://")).unwrap_or(false)
        }
    }
}

/// `__zippHostCall("zipp", op, ...)`.
pub(crate) fn host_call(ctx: &mut dyn HostCtx, args: &[String]) -> Result<String, String> {
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
    let num = |i: usize| arg(i).parse::<u64>().map_err(|_| format!("TypeError: zipp: argument {i} is not a number"));
    if arg(0) == "allowed" {
        return Ok(if allowed() { "1" } else { "0" }.into());
    }
    if !allowed() {
        return Err("SecurityError: this page may not use navigator.zipp".into());
    }
    let path = |i: usize| -> Result<PathBuf, String> {
        let p = PathBuf::from(arg(i));
        if p.is_absolute() { Ok(p) } else { Err(format!("TypeError: {:?} is not an absolute path", arg(i))) }
    };
    match arg(0) {
        "stat" => {
            let p = path(1)?;
            let meta = std::fs::symlink_metadata(&p).map_err(|e| io_error(&p, e))?;
            Ok(entry_json(None, &meta))
        }
        "list" => {
            let p = path(1)?;
            let mut entries: Vec<(String, String)> = Vec::new();
            for entry in std::fs::read_dir(&p).map_err(|e| io_error(&p, e))? {
                let entry = entry.map_err(|e| io_error(&p, e))?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Ok(meta) = entry.metadata().or_else(|_| std::fs::symlink_metadata(entry.path())) {
                    entries.push((name.clone(), entry_json(Some(&name), &meta)));
                }
            }
            entries.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
            Ok(format!("[{}]", entries.into_iter().map(|(_, j)| j).collect::<Vec<_>>().join(",")))
        }
        // `length` bytes from `offset` into __zippHostBytes; how many there were.
        "read" => {
            use std::io::{Read, Seek, SeekFrom};
            let (p, offset, length) = (path(1)?, num(2)?, num(3)? as usize);
            let (addr, count, kind) = ctx.typed_array_region("__zippHostBytes")?;
            if kind != 1 || count < length {
                return Err("TypeError: zipp: __zippHostBytes is too small".into());
            }
            // SAFETY: the engine pinned `count` bytes at `addr` for this call.
            let buf = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, length) };
            let mut file = std::fs::File::open(&p).map_err(|e| io_error(&p, e))?;
            file.seek(SeekFrom::Start(offset)).map_err(|e| io_error(&p, e))?;
            let mut got = 0;
            while got < length {
                match file.read(&mut buf[got..]) {
                    Ok(0) => break,
                    Ok(n) => got += n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(io_error(&p, e)),
                }
            }
            Ok(got.to_string())
        }
        // The first `length` bytes of __zippHostBytes, as the file or after it.
        "write" | "append" => {
            use std::io::Write;
            let (p, length) = (path(1)?, num(2)? as usize);
            let bytes = if length == 0 {
                Vec::new()
            } else {
                let (addr, count, kind) = ctx.typed_array_region("__zippHostBytes")?;
                if kind != 1 || count < length {
                    return Err("TypeError: zipp: __zippHostBytes does not hold the bytes".into());
                }
                // SAFETY: as above; copied out before anything else runs.
                unsafe { std::slice::from_raw_parts(addr as *const u8, length) }.to_vec()
            };
            let append = arg(0) == "append";
            changing(&p, || {
                let mut file = std::fs::OpenOptions::new().write(true).create(true).append(append).truncate(!append).open(&p)?;
                file.write_all(&bytes)?;
                file.sync_all()
            })
            .map_err(|e| io_error(&p, e))?;
            Ok(String::new())
        }
        "mkdir" => {
            let p = path(1)?;
            changing(&p, || std::fs::create_dir_all(&p)).map_err(|e| io_error(&p, e))?;
            Ok(String::new())
        }
        "remove" => {
            let (p, recursive) = (path(1)?, arg(2) == "1");
            changing(&p, || {
                let meta = std::fs::symlink_metadata(&p)?;
                match (meta.is_dir(), recursive) {
                    (true, true) => std::fs::remove_dir_all(&p),
                    (true, false) => std::fs::remove_dir(&p),
                    _ => std::fs::remove_file(&p),
                }
            })
            .map_err(|e| io_error(&p, e))?;
            Ok(String::new())
        }
        "rename" => {
            let (from, to) = (path(1)?, path(2)?);
            // Across drives a rename cannot be done: a copy, then the original gone.
            changing(&to, || match std::fs::rename(&from, &to) {
                Err(e) if e.raw_os_error() == Some(18) => copy_tree(&from, &to).and_then(|_| remove_tree(&from)),
                other => other,
            })
            .map_err(|e| io_error(&both(&from, &to), e))?;
            Ok(String::new())
        }
        "copy" => {
            let (from, to) = (path(1)?, path(2)?);
            changing(&to, || copy_tree(&from, &to)).map_err(|e| io_error(&both(&from, &to), e))?;
            Ok(String::new())
        }
        "drives" => drives(),
        "info" => Ok(info()),
        "network" => Ok(network()),
        // `run id program argsJSON cwd`: settles later, by id.
        "run" => {
            let id = num(1)?;
            let program = arg(2).to_string();
            let argv: Vec<String> = serde_json::from_str(arg(3)).map_err(|e| format!("TypeError: zipp: arguments: {e}"))?;
            let cwd = (!arg(4).is_empty()).then(|| arg(4).to_string());
            let page = PAGE.load(std::sync::atomic::Ordering::SeqCst);
            let _ = std::thread::Builder::new().name("zipp-run".into()).spawn(move || {
                let mut command = std::process::Command::new(&program);
                command.args(&argv).stdin(std::process::Stdio::null());
                if let Some(cwd) = cwd {
                    command.current_dir(cwd);
                }
                let result = match command.output() {
                    Ok(out) => serde_json::json!({
                        "code": out.status.code(),
                        "stdout": String::from_utf8_lossy(&out.stdout),
                        "stderr": String::from_utf8_lossy(&out.stderr),
                    }),
                    Err(e) => serde_json::json!({ "error": format!("NotFoundError: {program}: {e}") }),
                };
                let mut completions = COMPLETIONS.lock().unwrap_or_else(|e| e.into_inner());
                if PAGE.load(std::sync::atomic::Ordering::SeqCst) == page {
                    completions.push((id, result.to_string()));
                    drop(completions);
                    crate::worker::wake();
                }
            });
            Ok(String::new())
        }
        "power" => power(arg(1)),
        // A program on a terminal of its own (pty.rs): `spawn id program
        // argsJSON optionsJSON`, then its input, size and signals by id.
        #[cfg(target_os = "linux")]
        "spawn" => {
            let argv: Vec<String> = serde_json::from_str(arg(3)).map_err(|e| format!("TypeError: zipp: arguments: {e}"))?;
            let options = crate::pty::Options::from_json(arg(4))?;
            crate::pty::spawn(num(1)?, arg(2), &argv, options).map(|pid| pid.to_string())
        }
        #[cfg(target_os = "linux")]
        "pty_write" => crate::pty::write(num(1)?, arg(2)).map(|_| String::new()),
        #[cfg(target_os = "linux")]
        "pty_resize" => crate::pty::resize(num(1)?, arg(2).parse().unwrap_or(80), arg(3).parse().unwrap_or(24)).map(|_| String::new()),
        #[cfg(target_os = "linux")]
        "pty_kill" => crate::pty::kill(num(1)?, arg(2).parse().unwrap_or(libc::SIGTERM)).map(|_| String::new()),
        // The Alpine system on the boot drive's second partition, mounted.
        #[cfg(target_os = "linux")]
        "linux_root" => crate::pty::linux_root(),
        "wifi" => wifi(arg(1), arg(2), arg(3)),
        op => Err(format!("NotSupportedError: navigator.zipp has no operation {op}")),
    }
}

/// An error as the page sees it: a DOMException's name, then the message.
fn io_error(path: &Path, e: std::io::Error) -> String {
    use std::io::ErrorKind as K;
    let name = match e.kind() {
        K::NotFound => "NotFoundError",
        K::PermissionDenied => "NotAllowedError",
        K::AlreadyExists => "InvalidModificationError",
        _ if e.raw_os_error() == Some(39) => "InvalidModificationError", // a directory not empty
        _ => "OperationError",
    };
    format!("{name}: {}: {e}", path.display())
}

/// "from -> to", for an error that may be either's.
fn both(from: &Path, to: &Path) -> PathBuf {
    PathBuf::from(format!("{} -> {}", from.display(), to.display()))
}

fn entry_json(name: Option<&str>, meta: &std::fs::Metadata) -> String {
    let kind = if meta.is_dir() {
        "directory"
    } else if meta.is_file() {
        "file"
    } else if meta.file_type().is_symlink() {
        "link"
    } else {
        "other"
    };
    let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64);
    let mut json = serde_json::json!({ "kind": kind, "size": meta.len(), "modified": modified });
    if let Some(name) = name {
        json["name"] = name.into();
    }
    json.to_string()
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::metadata(from)?.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

fn remove_tree(p: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(p)?.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }
}

/// Make a change at `p`. On the boot drive, which is mounted read-only, the
/// drive is writable for just as long as that takes.
fn changing<T>(p: &Path, change: impl FnOnce() -> std::io::Result<T>) -> std::io::Result<T> {
    #[cfg(target_os = "linux")]
    if p.starts_with("/boot") {
        return boot::writable(change);
    }
    let _ = p;
    change()
}

#[cfg(target_os = "linux")]
mod boot {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;

    /// zipp-init takes the same lock to write its log.
    const LOCK: &str = "/run/zipp-boot.lock";

    pub(super) fn writable<T>(change: impl FnOnce() -> std::io::Result<T>) -> std::io::Result<T> {
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(LOCK)?;
        // SAFETY: a plain flock on a file this function holds open.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let device = boot_device().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no boot drive is mounted"))?;
        remount(&device, false)?;
        let result = change();
        // SAFETY: sync takes no arguments.
        unsafe { libc::sync() };
        let _ = remount(&device, true);
        result
    }

    /// The device mounted at /boot.
    fn boot_device() -> Option<String> {
        let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
        mounts.lines().find_map(|line| {
            let mut f = line.split_whitespace();
            let (device, target) = (f.next()?, f.next()?);
            (target == "/boot").then(|| device.to_string())
        })
    }

    fn remount(device: &str, read_only: bool) -> std::io::Result<()> {
        let source = CString::new(device).map_err(std::io::Error::other)?;
        let target = CString::new("/boot").unwrap_or_default();
        let fs = CString::new("vfat").unwrap_or_default();
        let mut flags = libc::MS_REMOUNT | libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC;
        if read_only {
            flags |= libc::MS_RDONLY;
        }
        // SAFETY: valid C strings for the call's duration; no data argument.
        if unsafe { libc::mount(source.as_ptr(), target.as_ptr(), fs.as_ptr(), flags, std::ptr::null()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

/// The mounted drives worth showing: the boot drive, USB drives and disks
/// (not the system's own filesystems), with their size and free space.
fn drives() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        let mounts = std::fs::read_to_string("/proc/mounts").map_err(|e| format!("OperationError: {e}"))?;
        let mut out = Vec::new();
        for line in mounts.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 4 || !f[0].starts_with("/dev/") {
                continue;
            }
            let (device, path, fs, options) = (f[0], f[1].replace("\\040", " "), f[2], f[3]);
            let (size, free) = space(&path).unwrap_or((0, 0));
            let name = device.trim_start_matches("/dev/");
            // The whole disk a partition is on: sda for sda1, mmcblk0 for mmcblk0p1.
            let disk = std::fs::read_link(format!("/sys/class/block/{name}"))
                .ok()
                .and_then(|l| l.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()))
                .filter(|d| d != "block")
                .unwrap_or_else(|| name.to_string());
            let removable = std::fs::read_to_string(format!("/sys/block/{disk}/removable")).is_ok_and(|s| s.trim() == "1")
                || std::fs::read_link(format!("/sys/class/block/{name}")).is_ok_and(|l| l.to_string_lossy().contains("/usb"));
            out.push(serde_json::json!({
                "path": path,
                "device": device,
                "label": label(device, fs),
                "filesystem": fs,
                "size": size,
                "free": free,
                "readOnly": options.split(',').any(|o| o == "ro") && path != "/boot",
                "boot": path == "/boot",
                "removable": removable,
            }));
        }
        Ok(serde_json::Value::Array(out).to_string())
    }
    #[cfg(not(target_os = "linux"))]
    Err("NotSupportedError: drives are listed on Linux only".into())
}

#[cfg(target_os = "linux")]
fn space(path: &str) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a valid path and a statvfs to fill.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    Some((s.f_blocks as u64 * s.f_frsize as u64, s.f_bavail as u64 * s.f_frsize as u64))
}

/// A FAT or ext2/3/4 volume's label, from its first sectors.
#[cfg(target_os = "linux")]
fn label(device: &str, fs: &str) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(device) else { return String::new() };
    let (at, len) = match fs {
        "vfat" => {
            let mut boot = [0u8; 512];
            if file.read_exact(&mut boot).is_err() {
                return String::new();
            }
            // FAT32 keeps it at 0x47, FAT12/16 at 0x2b.
            let fat32 = u16::from_le_bytes([boot[0x16], boot[0x17]]) == 0;
            let at = if fat32 { 0x47 } else { 0x2b };
            let text = String::from_utf8_lossy(&boot[at..at + 11]).trim().to_string();
            return if text == "NO NAME" { String::new() } else { text };
        }
        "ext2" | "ext3" | "ext4" => (1024 + 0x78, 16),
        _ => return String::new(),
    };
    let mut buf = vec![0u8; len];
    if file.seek(SeekFrom::Start(at)).is_err() || file.read_exact(&mut buf).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&buf).trim_end_matches('\0').trim().to_string()
}

/// The machine: name, kernel, processor, memory, uptime.
fn info() -> String {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default().trim().trim_end_matches('\0').to_string();
    let meminfo = read("/proc/meminfo");
    let mem = |key: &str| {
        meminfo
            .lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(0, |kb| kb * 1024)
    };
    let model = [read("/proc/device-tree/model"), format!("{} {}", read("/sys/class/dmi/id/sys_vendor"), read("/sys/class/dmi/id/product_name"))]
        .into_iter()
        .find(|m| !m.trim().is_empty())
        .unwrap_or_default();
    let cpu = read("/proc/cpuinfo")
        .lines()
        .find(|l| l.starts_with("model name") || l.starts_with("Model"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_default();
    serde_json::json!({
        "hostname": read("/proc/sys/kernel/hostname"),
        "kernel": read("/proc/sys/kernel/osrelease"),
        "arch": std::env::consts::ARCH,
        "model": model.trim(),
        "cpu": cpu,
        "cpus": std::thread::available_parallelism().map_or(1, |n| n.get()),
        "uptime": read("/proc/uptime").split_whitespace().next().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0),
        "memory": { "total": mem("MemTotal:"), "available": mem("MemAvailable:") },
    })
    .to_string()
}

/// Every interface with its state and addresses, the default routes and the
/// name servers.
fn network() -> String {
    let read = |p: String| std::fs::read_to_string(p).unwrap_or_default().trim().to_string();
    let mut interfaces = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/sys/class/net") {
        let mut names: Vec<String> = dir.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for name in names {
            let base = format!("/sys/class/net/{name}");
            let wireless = Path::new(&format!("{base}/wireless")).exists();
            let kind = match read(format!("{base}/type")).as_str() {
                "1" if wireless => "wireless",
                "1" => "wired",
                "772" => "loopback",
                _ => "other",
            };
            interfaces.push(serde_json::json!({
                "name": name,
                "kind": kind,
                "state": read(format!("{base}/operstate")),
                "mac": read(format!("{base}/address")),
                "addresses": addresses(&name),
            }));
        }
    }
    let routes: Vec<serde_json::Value> = std::fs::read_to_string("/proc/net/route")
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 8 || f[1] != "00000000" || f[7] != "00000000" {
                return None;
            }
            let gateway = std::net::Ipv4Addr::from(u32::from_str_radix(f[2], 16).ok()?.swap_bytes());
            Some(serde_json::json!({ "interface": f[0], "gateway": gateway.to_string(), "metric": f[6].parse::<u32>().unwrap_or(0) }))
        })
        .collect();
    let dns: Vec<String> = std::fs::read_to_string("/etc/resolv.conf")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("nameserver ").map(|s| s.trim().to_string()))
        .collect();
    serde_json::json!({ "interfaces": interfaces, "routes": routes, "dns": dns }).to_string()
}

/// An interface's IPv4 and IPv6 addresses, with their prefix lengths.
fn addresses(name: &str) -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        let mut out = Vec::new();
        let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
        // SAFETY: getifaddrs fills a list freed below; each node is read only
        // while the list lives.
        unsafe {
            if libc::getifaddrs(&mut list) != 0 {
                return out;
            }
            let mut node = list;
            while !node.is_null() {
                let ifa = &*node;
                node = ifa.ifa_next;
                if ifa.ifa_addr.is_null() || std::ffi::CStr::from_ptr(ifa.ifa_name).to_str() != Ok(name) {
                    continue;
                }
                match i32::from((*ifa.ifa_addr).sa_family) {
                    libc::AF_INET => {
                        let a = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                        let addr = std::net::Ipv4Addr::from(u32::from_be(a.sin_addr.s_addr));
                        let prefix = if ifa.ifa_netmask.is_null() {
                            32
                        } else {
                            u32::from_be((*(ifa.ifa_netmask as *const libc::sockaddr_in)).sin_addr.s_addr).count_ones()
                        };
                        out.push(format!("{addr}/{prefix}"));
                    }
                    libc::AF_INET6 => {
                        let a = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                        let addr = std::net::Ipv6Addr::from(a.sin6_addr.s6_addr);
                        let prefix = if ifa.ifa_netmask.is_null() {
                            128
                        } else {
                            (*(ifa.ifa_netmask as *const libc::sockaddr_in6)).sin6_addr.s6_addr.iter().map(|b| b.count_ones()).sum()
                        };
                        out.push(format!("{addr}/{prefix}"));
                    }
                    _ => {}
                }
            }
            libc::freeifaddrs(list);
        }
        out
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = name;
        Vec::new()
    }
}

/// Wi-Fi, through the wpa_supplicant zipp-linux's init runs on the wireless
/// card: `status`, `scan` (asks for one; `results` some seconds later),
/// `results`, `connect ssid passphrase` (none for an open network), and
/// `disconnect`.
fn wifi(op: &str, ssid: &str, passphrase: &str) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        let iface = wpa::interface().ok_or("NotFoundError: there is no Wi-Fi (no wireless card, or wifi=off)")?;
        let ask = |command: &str| wpa::request(&iface, command).map_err(|e| format!("OperationError: wpa_supplicant: {e}"));
        let ok = |command: &str| -> Result<(), String> {
            let reply = ask(command)?;
            if reply.trim() == "OK" {
                Ok(())
            } else {
                Err(format!("OperationError: wpa_supplicant: {} ({})", reply.trim(), command.split(' ').next().unwrap_or("")))
            }
        };
        match op {
            "status" => {
                let reply = ask("STATUS")?;
                let field = |key: &str| reply.lines().find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('='))).unwrap_or("").to_string();
                Ok(serde_json::json!({
                    "interface": iface,
                    "state": field("wpa_state"),
                    "ssid": wpa::unescape(&field("ssid")),
                    "bssid": field("bssid"),
                    "frequency": field("freq").parse::<u32>().unwrap_or(0),
                    "address": field("ip_address"),
                    "security": field("key_mgmt"),
                })
                .to_string())
            }
            "scan" => {
                let reply = ask("SCAN")?;
                // Already scanning is as good as starting.
                if reply.trim() != "OK" && !reply.contains("FAIL-BUSY") {
                    return Err(format!("OperationError: wpa_supplicant: {}", reply.trim()));
                }
                Ok(String::new())
            }
            "results" => {
                let reply = ask("SCAN_RESULTS")?;
                // bssid, frequency, signal, flags, ssid; one network may have
                // several access points: the strongest stands for it.
                let mut best: Vec<serde_json::Value> = Vec::new();
                for line in reply.lines().skip(1) {
                    let f: Vec<&str> = line.splitn(5, '\t').collect();
                    if f.len() < 5 || f[4].is_empty() {
                        continue;
                    }
                    let signal: i64 = f[2].parse().unwrap_or(-100);
                    let flags = f[3];
                    let security = if flags.contains("SAE") {
                        "WPA3"
                    } else if flags.contains("EAP") {
                        "enterprise"
                    } else if flags.contains("WPA2") || flags.contains("RSN") {
                        "WPA2"
                    } else if flags.contains("WPA") {
                        "WPA"
                    } else if flags.contains("WEP") {
                        "WEP"
                    } else {
                        "open"
                    };
                    let entry = serde_json::json!({
                        "ssid": wpa::unescape(f[4]),
                        "signal": signal,
                        "frequency": f[1].parse::<u32>().unwrap_or(0),
                        "security": security,
                        "bssid": f[0],
                    });
                    match best.iter_mut().find(|e| e["ssid"] == entry["ssid"]) {
                        Some(e) if e["signal"].as_i64().unwrap_or(-100) < signal => *e = entry,
                        Some(_) => {}
                        None => best.push(entry),
                    }
                }
                best.sort_by_key(|e| -e["signal"].as_i64().unwrap_or(-100));
                Ok(serde_json::Value::Array(best).to_string())
            }
            "connect" => {
                if ssid.is_empty() || ssid.len() > 32 {
                    return Err("TypeError: a network's name is 1 to 32 bytes".into());
                }
                let hex = passphrase.len() == 64 && passphrase.bytes().all(|b| b.is_ascii_hexdigit());
                if !passphrase.is_empty() && !hex && !(8..=63).contains(&passphrase.len()) {
                    return Err("TypeError: a passphrase is 8 to 63 characters".into());
                }
                ok("REMOVE_NETWORK all")?;
                let id = ask("ADD_NETWORK")?.trim().to_string();
                if id.parse::<u32>().is_err() {
                    return Err(format!("OperationError: wpa_supplicant: {id}"));
                }
                let name: String = ssid.bytes().map(|b| format!("{b:02x}")).collect();
                ok(&format!("SET_NETWORK {id} ssid {name}"))?;
                ok(&format!("SET_NETWORK {id} scan_ssid 1"))?;
                if passphrase.is_empty() {
                    ok(&format!("SET_NETWORK {id} key_mgmt NONE"))?;
                } else if hex {
                    ok(&format!("SET_NETWORK {id} key_mgmt WPA-PSK WPA-PSK-SHA256"))?;
                    ok(&format!("SET_NETWORK {id} psk {passphrase}"))?;
                } else {
                    ok(&format!("SET_NETWORK {id} key_mgmt WPA-PSK WPA-PSK-SHA256 SAE"))?;
                    ok(&format!("SET_NETWORK {id} ieee80211w 1"))?;
                    ok(&format!("SET_NETWORK {id} psk \"{passphrase}\""))?;
                }
                ok(&format!("SELECT_NETWORK {id}"))?;
                Ok(String::new())
            }
            "disconnect" => {
                ok("DISCONNECT")?;
                Ok(String::new())
            }
            _ => Err(format!("TypeError: zipp: wifi {op}?")),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (op, ssid, passphrase);
        Err("NotSupportedError: Wi-Fi is for zipp-linux".into())
    }
}

/// wpa_supplicant's control interface: a datagram socket per interface in
/// /run/wpa_supplicant, a request and its reply.
#[cfg(target_os = "linux")]
mod wpa {
    use std::os::unix::net::UnixDatagram;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    const DIR: &str = "/run/wpa_supplicant";

    /// The interface wpa_supplicant runs on (not the Wi-Fi Direct device it
    /// makes beside it, `p2p-dev-<interface>`).
    pub(super) fn interface() -> Option<String> {
        let mut names: Vec<String> = std::fs::read_dir(DIR)
            .ok()?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| !name.starts_with("p2p-dev-"))
            .collect();
        names.sort();
        names.into_iter().next()
    }

    pub(super) fn request(iface: &str, command: &str) -> std::io::Result<String> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let local = format!("/run/zipp-wpa-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let _ = std::fs::remove_file(&local);
        let socket = UnixDatagram::bind(&local)?;
        let result = (|| {
            socket.connect(format!("{DIR}/{iface}"))?;
            socket.set_read_timeout(Some(Duration::from_secs(4)))?;
            socket.send(command.as_bytes())?;
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = socket.recv(&mut buf)?;
                let reply = String::from_utf8_lossy(&buf[..n]).into_owned();
                // "<3>CTRL-EVENT-..." is an event, not the answer.
                if !reply.starts_with('<') {
                    return Ok(reply);
                }
            }
        })();
        let _ = std::fs::remove_file(&local);
        result
    }

    /// A name as wpa_supplicant prints it (\xNN, \\, \") as text.
    pub(super) fn unescape(text: &str) -> String {
        let mut bytes = Vec::with_capacity(text.len());
        let mut it = text.bytes();
        while let Some(b) = it.next() {
            if b != b'\\' {
                bytes.push(b);
                continue;
            }
            match it.next() {
                Some(b'x') => {
                    let hex: Vec<u8> = it.by_ref().take(2).collect();
                    match std::str::from_utf8(&hex).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                        Some(v) => bytes.push(v),
                        None => bytes.extend_from_slice(&hex),
                    }
                }
                Some(b'n') => bytes.push(b'\n'),
                Some(b't') => bytes.push(b'\t'),
                Some(other) => bytes.push(other),
                None => bytes.push(b'\\'),
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// Reboot or power off, the way busybox's `reboot` asks zipp-init (PID 1).
fn power(how: &str) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        if !KIOSK.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("NotAllowedError: only the system's own browser (--kiosk) may restart the machine".into());
        }
        let signal = match how {
            "reboot" => libc::SIGTERM,
            "off" => libc::SIGUSR2,
            _ => return Err(format!("TypeError: zipp: power {how}?")),
        };
        // SAFETY: a signal to init.
        unsafe { libc::kill(1, signal) };
        Ok(String::new())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = how;
        Err("NotSupportedError: power is for zipp-linux".into())
    }
}
