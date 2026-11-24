//! Programs on a terminal of their own (navigator.zipp.system.spawn): a
//! pseudo-terminal the page reads and writes, so interactive programs --
//! a shell, an editor, `top` -- run as they would on a console.
//!
//! A program may be started in a jail: its own mount namespace with `/proc`,
//! `/dev` and `/sys` mounted into a root of its own, chrooted there, and
//! running as an ordinary user with no way to gain privileges back
//! (`PR_SET_NO_NEW_PRIVS`). That is how ZIPP OS runs the Linux apps of the
//! Alpine system on the boot drive's second partition (`linux_root`).
//!
//! What a program prints is read on a thread of its own and handed to the
//! page through the event bridge (`__zippPtyEvent`), whole characters only;
//! what the page types is written on another, so a program that does not
//! read never holds up the window. A new page ends every program the last
//! one started.

use std::collections::HashMap;
use std::ffi::CString;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Condvar, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};

struct Pty {
    master: OwnedFd,
    pid: i32,
    input: SyncSender<Vec<u8>>,
}

struct EventQueue {
    events: Vec<(u64, &'static str, String)>,
    bytes: usize,
}

const MAX_PTY_BUFFERED_BYTES: usize = 1024 * 1024;
const MAX_DRAIN_BYTES: usize = 256 * 1024;

static PTYS: Mutex<Option<HashMap<u64, Pty>>> = Mutex::new(None);
/// (id, "data" | "exit", text or exit code), for the page, with byte-budget backpressure.
static EVENTS: (Mutex<EventQueue>, Condvar) = (
    Mutex::new(EventQueue { events: Vec::new(), bytes: 0 }),
    Condvar::new(),
);
static PAGE: AtomicU64 = AtomicU64::new(0);

/// What has happened on the page's terminals since last asked.
pub(crate) fn take_events() -> Vec<(u64, &'static str, String)> {
    let (lock, cvar) = &EVENTS;
    let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
    if q.events.is_empty() {
        return Vec::new();
    }
    let mut taken = Vec::new();
    let mut drained_bytes = 0usize;
    let mut split_at = 0usize;
    for (i, ev) in q.events.iter().enumerate() {
        split_at = i + 1;
        drained_bytes += ev.2.len();
        if drained_bytes >= MAX_DRAIN_BYTES {
            break;
        }
    }
    taken.extend(q.events.drain(..split_at));
    q.bytes = q.bytes.saturating_sub(drained_bytes);
    let more_left = !q.events.is_empty();
    cvar.notify_all();
    drop(q);
    if more_left {
        crate::worker::wake();
    }
    // Output of one terminal that came in pieces goes to the page as one.
    let mut merged: Vec<(u64, &'static str, String)> = Vec::with_capacity(taken.len());
    for event in taken {
        match merged.last_mut() {
            Some(last) if last.0 == event.0 && last.1 == "data" && event.1 == "data" => last.2.push_str(&event.2),
            _ => merged.push(event),
        }
    }
    merged
}

/// A new page: the last one's programs are ended (their whole process
/// groups), and what they printed is no one's.
pub(crate) fn new_page() {
    PAGE.fetch_add(1, Ordering::SeqCst);
    let ptys = PTYS.lock().unwrap_or_else(|e| e.into_inner()).take();
    for pty in ptys.into_iter().flat_map(|p| p.into_values()) {
        // SAFETY: signals to a process group this module started.
        unsafe {
            libc::kill(-pty.pid, libc::SIGHUP);
            libc::kill(-pty.pid, libc::SIGTERM);
        }
    }
    let (lock, cvar) = &EVENTS;
    let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
    q.events.clear();
    q.bytes = 0;
    cvar.notify_all();
}

fn push(page: u64, id: u64, kind: &'static str, text: String) {
    let (lock, cvar) = &EVENTS;
    let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
    // Apply backpressure to runaway stdout (`yes`, `find /`) so the desktop cannot OOM.
    if kind == "data" {
        while q.bytes >= MAX_PTY_BUFFERED_BYTES && PAGE.load(Ordering::SeqCst) == page {
            q = cvar.wait(q).unwrap_or_else(|e| e.into_inner());
        }
    }
    if PAGE.load(Ordering::SeqCst) != page {
        return;
    }
    let was_empty = q.events.is_empty();
    q.bytes += text.len();
    q.events.push((id, kind, text));
    drop(q);
    if was_empty {
        crate::worker::wake();
    }
}

/// How many leading bytes of `carry` end on a complete UTF-8 boundary, holding
/// back only a trailing valid-prefix incomplete sequence (1..=3 bytes).
fn utf8_whole_prefix(carry: &[u8]) -> usize {
    match std::str::from_utf8(carry) {
        Ok(_) => carry.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => {
            // Invalid bytes exist earlier, but the very tail may still be a split multibyte sequence.
            let tail_start = carry.len().saturating_sub(3);
            for i in (tail_start..carry.len()).rev() {
                let b = carry[i];
                if (b & 0xC0) != 0x80 {
                    let expected = if b < 0x80 {
                        1
                    } else if (b & 0xE0) == 0xC0 {
                        2
                    } else if (b & 0xF0) == 0xE0 {
                        3
                    } else if (b & 0xF8) == 0xF0 {
                        4
                    } else {
                        return carry.len();
                    };
                    if carry.len() - i < expected {
                        return i;
                    }
                    break;
                }
            }
            carry.len()
        }
    }
}

/// How a program is to be started: `{ cwd, cols, rows, env: {..}, jail:
/// { root, uid, gid } }`, every part optional.
pub(crate) struct Options {
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    env: HashMap<String, String>,
    /// Run it in this root, as this user (see the module docs).
    jail: Option<Jail>,
    /// Run it as this user, on this system: a program that has no need to be
    /// root, and that shows what the web sends it (a browser in a window).
    user: Option<u32>,
}

pub(crate) struct Jail {
    root: String,
    uid: u32,
    gid: u32,
}

impl Options {
    pub(crate) fn from_json(text: &str) -> Result<Options, String> {
        let v: serde_json::Value = if text.is_empty() { serde_json::json!({}) } else { serde_json::from_str(text).map_err(|e| format!("TypeError: options: {e}"))? };
        let num = |v: &serde_json::Value| v.as_u64().map(|n| n.min(u16::MAX as u64) as u16);
        Ok(Options {
            cwd: v["cwd"].as_str().filter(|s| !s.is_empty()).map(String::from),
            cols: num(&v["cols"]),
            rows: num(&v["rows"]),
            env: v["env"].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default(),
            jail: v["jail"]["root"].as_str().map(|root| Jail {
                root: root.to_string(),
                uid: v["jail"]["uid"].as_u64().unwrap_or(0) as u32,
                gid: v["jail"]["gid"].as_u64().unwrap_or(0) as u32,
            }),
            user: v["user"].as_u64().filter(|n| (1..=u32::MAX as u64).contains(n)).map(|n| n as u32),
        })
    }
}

/// Start `program` on a terminal of its own; its process id.
pub(crate) fn spawn(id: u64, program: &str, args: &[String], options: Options) -> Result<i32, String> {
    let cols = options.cols.unwrap_or(80).max(2);
    let rows = options.rows.unwrap_or(24).max(2);
    let (mut master, mut slave) = (0, 0);
    let size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: openpty fills the two descriptors.
    if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), &size) } != 0 {
        return Err(format!("OperationError: no terminal: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: both descriptors were just opened and are owned here.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    // Neither fd leaks into unrelated concurrent forks; Stdio::from dups slave onto 0/1/2 in the child.
    unsafe {
        libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(slave.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
    }
    // Clone reader/writer handles before spawning so clone failure cannot leak a child.
    let writer = master.try_clone().map_err(|e| e.to_string())?;
    let reader = master.try_clone().map_err(|e| e.to_string())?;

    let mut command = std::process::Command::new(program);
    command.args(args).env_clear();
    command.env("TERM", "xterm-256color").env("COLORTERM", "truecolor").env("LANG", "C.UTF-8");
    command.env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin");
    for (k, v) in &options.env {
        command.env(k, v);
    }
    let io = |fd: &OwnedFd| fd.try_clone().map(std::process::Stdio::from).map_err(|e| e.to_string());
    command.stdin(io(&slave)?).stdout(io(&slave)?).stderr(io(&slave)?);

    // Everything the child needs is made here, before it is forked: after
    // the fork it may only make system calls.
    let user = options.user;
    let jail = match &options.jail {
        Some(j) => Some(JailPaths::new(j, options.cwd.as_deref())?),
        None => {
            if let Some(cwd) = &options.cwd {
                command.current_dir(cwd);
            }
            None
        }
    };
    // SAFETY: the closure only makes system calls, on memory made above.
    unsafe {
        command.pre_exec(move || {
            // A session of its own, with this terminal as its controlling one.
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            // The desktop is the last thing the kernel kills for memory
            // (zipp-init); what it starts is among the first, and a window
            // that runs away costs that window.
            let adj = libc::open(c"/proc/self/oom_score_adj".as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
            if adj >= 0 {
                libc::write(adj, c"300".as_ptr().cast(), 3);
                libc::close(adj);
            }
            if let Some(jail) = &jail {
                jail.enter()?;
            }
            if let Some(uid) = user {
                drop_to(uid)?;
            }
            Ok(())
        });
    }
    let child = command.spawn().map_err(|e| format!("NotFoundError: {program}: {e}"))?;
    drop(command);
    drop(slave);
    let pid = child.id() as i32;
    let page = PAGE.load(Ordering::SeqCst);

    // What the page types, written on a thread of its own with write batching.
    let (tx, rx) = sync_channel::<Vec<u8>>(256);
    if let Err(e) = std::thread::Builder::new().name("pty-in".into()).spawn(move || {
        let mut file = std::fs::File::from(writer);
        let mut batch = Vec::new();
        while let Ok(bytes) = rx.recv() {
            batch.clear();
            batch.extend_from_slice(&bytes);
            while batch.len() < 16 * 1024 {
                match rx.try_recv() {
                    Ok(more) => batch.extend_from_slice(&more),
                    Err(_) => break,
                }
            }
            if file.write_all(&batch).is_err() {
                break;
            }
        }
    }) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
            let mut status = 0;
            libc::waitpid(pid, &mut status, 0);
        }
        std::mem::forget(child);
        return Err(format!("OperationError: pty-in thread: {e}"));
    }
    // What the program prints, read on another, and its end.
    if let Err(e) = std::thread::Builder::new().name("pty-out".into()).spawn(move || {
        let mut file = std::fs::File::from(reader);
        let mut buf = vec![0u8; 16 * 1024];
        let mut carry: Vec<u8> = Vec::new();
        loop {
            match file.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    carry.extend_from_slice(&buf[..n]);
                    // Whole characters only: a character cut across two
                    // reads waits for the rest of it.
                    let whole = utf8_whole_prefix(&carry);
                    let text = String::from_utf8_lossy(&carry[..whole]).into_owned();
                    carry.drain(..whole);
                    if !text.is_empty() {
                        push(page, id, "data", text);
                    }
                }
            }
        }
        let mut status = 0;
        // SAFETY: the child this module started.
        unsafe { libc::waitpid(pid, &mut status, 0) };
        let code = if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { 128 + libc::WTERMSIG(status) };
        if let Some(ptys) = PTYS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            ptys.remove(&id);
        }
        push(page, id, "exit", code.to_string());
    }) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
            let mut status = 0;
            libc::waitpid(pid, &mut status, 0);
        }
        std::mem::forget(child);
        return Err(format!("OperationError: pty-out thread: {e}"));
    }
    // The child is waited for by the reader; std must not reap it first.
    std::mem::forget(child);
    PTYS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(id, Pty { master, pid, input: tx });
    Ok(pid)
}

/// Bytes for a terminal's program, as typed.
pub(crate) fn write(id: u64, text: &str) -> Result<(), String> {
    let ptys = PTYS.lock().unwrap_or_else(|e| e.into_inner());
    let pty = ptys.as_ref().and_then(|p| p.get(&id)).ok_or("InvalidStateError: that program has ended")?;
    let _ = pty.input.try_send(text.as_bytes().to_vec());
    Ok(())
}

/// A terminal's new size: its program hears of it (SIGWINCH).
pub(crate) fn resize(id: u64, cols: u16, rows: u16) -> Result<(), String> {
    let ptys = PTYS.lock().unwrap_or_else(|e| e.into_inner());
    let pty = ptys.as_ref().and_then(|p| p.get(&id)).ok_or("InvalidStateError: that program has ended")?;
    let size = libc::winsize { ws_row: rows.max(2), ws_col: cols.max(2), ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: the terminal's master, and a winsize.
    unsafe { libc::ioctl(pty.master.as_raw_fd(), libc::TIOCSWINSZ as _, &size) };
    Ok(())
}

/// A signal to a terminal's program and everything it started.
pub(crate) fn kill(id: u64, signal: i32) -> Result<(), String> {
    let ptys = PTYS.lock().unwrap_or_else(|e| e.into_inner());
    let pty = ptys.as_ref().and_then(|p| p.get(&id)).ok_or("InvalidStateError: that program has ended")?;
    // SAFETY: a signal to the process group this module started.
    unsafe { libc::kill(-pty.pid, signal) };
    Ok(())
}

/// A jail's paths and user, made ready before the fork.
/// Capabilities no program in a jail has: `CAP_KILL`, `CAP_NET_ADMIN`,
/// `CAP_SYS_MODULE`, `CAP_SYS_RAWIO`, `CAP_SYS_CHROOT`, `CAP_SYS_PTRACE`,
/// `CAP_SYS_ADMIN`, `CAP_SYS_BOOT`, `CAP_SYS_TIME`, `CAP_MKNOD`,
/// `CAP_MAC_ADMIN`, `CAP_MAC_OVERRIDE`, `CAP_SYSLOG`, `CAP_PERFMON`, `CAP_BPF`.
const JAIL_DROPS: [libc::c_ulong; 15] = [5, 12, 16, 17, 18, 19, 21, 22, 25, 27, 32, 33, 34, 38, 39];

struct JailPaths {
    root: CString,
    proc_: CString,
    dev: CString,
    sys: CString,
    cwd: CString,
    slash: CString,
    procfs: CString,
    host_sys: CString,
    has_render: bool,
    has_snd: bool,
    /// The Wayland server's directory (wayland.rs), and where it shows in
    /// the jail: the same /run/user/1000.
    wayland: Option<(CString, CString)>,
    uid: u32,
    gid: u32,
}

impl JailPaths {
    fn new(jail: &Jail, cwd: Option<&str>) -> Result<Self, String> {
        let c = |s: String| CString::new(s).map_err(|e| e.to_string());
        let root = jail.root.trim_end_matches('/').to_string();
        if !std::path::Path::new(&root).join("bin").exists() {
            return Err(format!("NotFoundError: {root} is not a Linux system"));
        }
        // Its own name servers: the system's /etc/resolv.conf is a link
        // into /run, which the jail does not see. Verify `{root}/etc` is a real
        // directory (never follow a jail-planted symlink out to host `/etc`).
        let etc_dir = std::path::Path::new(&root).join("etc");
        if std::fs::symlink_metadata(&etc_dir).is_ok_and(|m| m.is_dir())
            && let Ok(dns) = std::fs::read_to_string("/etc/resolv.conf")
        {
            let target = etc_dir.join("resolv.conf");
            let _ = std::fs::remove_file(&target);
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&target)
            {
                let _ = f.write_all(dns.as_bytes());
            }
        }
        for dir in ["proc", "dev", "sys", "tmp"] {
            let p = std::path::Path::new(&root).join(dir);
            if std::fs::symlink_metadata(&p).is_err() {
                let _ = std::fs::create_dir(&p);
            }
        }
        let wayland = if std::path::Path::new(crate::wayland::RUNTIME_DIR).is_dir() {
            let inside = format!("{root}{}", crate::wayland::RUNTIME_DIR);
            let _ = std::fs::create_dir_all(&inside);
            Some((c(crate::wayland::RUNTIME_DIR.into())?, c(inside)?))
        } else {
            None
        };
        Ok(JailPaths {
            proc_: c(format!("{root}/proc"))?,
            dev: c(format!("{root}/dev"))?,
            sys: c(format!("{root}/sys"))?,
            cwd: c(cwd.unwrap_or("/").to_string())?,
            root: c(root)?,
            slash: c("/".into())?,
            procfs: c("proc".into())?,
            host_sys: c("/sys".into())?,
            has_render: std::path::Path::new("/dev/dri/renderD128").exists(),
            has_snd: std::path::Path::new("/dev/snd").is_dir(),
            wayland,
            uid: jail.uid,
            gid: jail.gid,
        })
    }

    /// In the child: its own mounts, the root, the user.
    fn enter(&self) -> std::io::Result<()> {
        let check = |rc: libc::c_int| if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) };
        let none = std::ptr::null::<libc::c_char>();
        // SAFETY: system calls only, on static CStrs and CStrings built before fork.
        unsafe {
            check(libc::unshare(libc::CLONE_NEWNS))?;
            // Nothing mounted here reaches the system's own mounts.
            check(libc::mount(none, self.slash.as_ptr(), none, libc::MS_REC | libc::MS_PRIVATE, std::ptr::null()))?;
            check(libc::mount(self.procfs.as_ptr(), self.proc_.as_ptr(), self.procfs.as_ptr(), libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC, std::ptr::null()))?;

            // Minimal isolated `/dev`: expose only safe character devices, a fresh
            // `devpts`, `shm`, GPU render node, and audio — never host disks,
            // raw input devices, `/dev/tty0`, `/dev/mem`, or `/dev/vboxguest`.
            check(libc::mount(
                c"tmpfs".as_ptr(),
                self.dev.as_ptr(),
                c"tmpfs".as_ptr(),
                libc::MS_NOSUID | libc::MS_NOEXEC,
                c"mode=0755,size=16m".as_ptr().cast(),
            ))?;
            check(libc::chdir(self.dev.as_ptr()))?;
            let bind_dev = |host: &std::ffi::CStr, name: &std::ffi::CStr| {
                let fd = libc::open(name.as_ptr(), libc::O_CREAT | libc::O_WRONLY | libc::O_CLOEXEC, 0o666);
                if fd >= 0 {
                    libc::close(fd);
                    libc::mount(host.as_ptr(), name.as_ptr(), none, libc::MS_BIND, std::ptr::null());
                }
            };
            bind_dev(c"/dev/null", c"null");
            bind_dev(c"/dev/zero", c"zero");
            bind_dev(c"/dev/full", c"full");
            bind_dev(c"/dev/random", c"random");
            bind_dev(c"/dev/urandom", c"urandom");
            bind_dev(c"/dev/tty", c"tty");
            if libc::mkdir(c"pts".as_ptr(), 0o755) == 0 {
                if libc::mount(
                    c"devpts".as_ptr(),
                    c"pts".as_ptr(),
                    c"devpts".as_ptr(),
                    libc::MS_NOSUID | libc::MS_NOEXEC,
                    c"newinstance,ptmxmode=0666,mode=0620".as_ptr().cast(),
                ) == 0
                {
                    libc::symlink(c"pts/ptmx".as_ptr(), c"ptmx".as_ptr());
                } else {
                    bind_dev(c"/dev/ptmx", c"ptmx");
                }
            }
            if libc::mkdir(c"shm".as_ptr(), 0o1777) == 0 {
                libc::mount(
                    c"tmpfs".as_ptr(),
                    c"shm".as_ptr(),
                    c"tmpfs".as_ptr(),
                    libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
                    c"mode=1777".as_ptr().cast(),
                );
            }
            libc::symlink(c"/proc/self/fd".as_ptr(), c"fd".as_ptr());
            libc::symlink(c"/proc/self/fd/0".as_ptr(), c"stdin".as_ptr());
            libc::symlink(c"/proc/self/fd/1".as_ptr(), c"stdout".as_ptr());
            libc::symlink(c"/proc/self/fd/2".as_ptr(), c"stderr".as_ptr());
            if self.has_render && libc::mkdir(c"dri".as_ptr(), 0o755) == 0 {
                bind_dev(c"/dev/dri/renderD128", c"dri/renderD128");
            }
            if self.has_snd && libc::mkdir(c"snd".as_ptr(), 0o755) == 0 {
                libc::mount(c"/dev/snd".as_ptr(), c"snd".as_ptr(), none, libc::MS_BIND, std::ptr::null());
            }

            // Non-recursive bind of `/sys` + read-only remount excludes writable
            // host submounts (`/sys/fs/cgroup`, `efivars`, `pstore`).
            check(libc::mount(self.host_sys.as_ptr(), self.sys.as_ptr(), none, libc::MS_BIND, std::ptr::null()))?;
            check(libc::mount(none, self.sys.as_ptr(), none, libc::MS_REMOUNT | libc::MS_BIND | libc::MS_RDONLY, std::ptr::null()))?;
            if let Some((host, inside)) = &self.wayland {
                check(libc::mount(host.as_ptr(), inside.as_ptr(), none, libc::MS_BIND, std::ptr::null()))?;
            }
            check(libc::chroot(self.root.as_ptr()))?;
            if libc::chdir(self.cwd.as_ptr()) != 0 {
                check(libc::chdir(self.slash.as_ptr()))?;
            }
            // Root in here is for installing packages. It keeps what that
            // takes and not what reaches out of the jail: with these, /proc/1
            // was the desktop's whole system, and modules, raw disks and
            // mounts were all open to it.
            for cap in JAIL_DROPS {
                check(libc::prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0))?;
            }
            if self.uid != 0 {
                check(libc::setgroups(0, std::ptr::null()))?;
                check(libc::setgid(self.gid))?;
                check(libc::setuid(self.uid))?;
            }
            check(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))?;
        }
        Ok(())
    }
}

/// Become `uid` for good, in a group of the same number, in no others and with
/// no way back. Between fork and exec, so system calls only.
fn drop_to(uid: u32) -> std::io::Result<()> {
    let check = |rc: libc::c_int| if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) };
    // SAFETY: system calls on no memory of ours.
    unsafe {
        check(libc::setgroups(0, std::ptr::null()))?;
        check(libc::setgid(uid))?;
        check(libc::setuid(uid))?;
        check(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))?;
    }
    Ok(())
}

/// The Linux system on the boot drive's second partition (an ext4 one
/// labelled ZIPPOS-LINUX), mounted at /linux: where it is, or why not.
pub(crate) fn linux_root() -> Result<String, String> {
    const ROOT: &str = "/linux";
    if std::fs::read_to_string("/proc/mounts").is_ok_and(|m| m.lines().any(|l| l.split_whitespace().nth(1) == Some(ROOT))) {
        return Ok(ROOT.into());
    }
    let device = find_labelled("ZIPPOS-LINUX").ok_or("NotFoundError: there is no Linux partition (ZIPPOS-LINUX) on this machine")?;
    let _ = std::fs::create_dir_all(ROOT);
    let source = CString::new(format!("/dev/{device}")).map_err(|e| e.to_string())?;
    let target = CString::new(ROOT).map_err(|e| e.to_string())?;
    let fs = CString::new("ext4").map_err(|e| e.to_string())?;
    // No device files or set-uid programs from it: its files are its programs',
    // and the jail has its own /dev.
    let flags = libc::MS_NOATIME | libc::MS_NODEV | libc::MS_NOSUID;
    // SAFETY: valid strings for the call; no options.
    if unsafe { libc::mount(source.as_ptr(), target.as_ptr(), fs.as_ptr(), flags, std::ptr::null()) } != 0 {
        return Err(format!("OperationError: cannot mount the Linux partition: {}", std::io::Error::last_os_error()));
    }
    Ok(ROOT.into())
}

/// The whole disk a partition is on (`sda` for `sda2`), as sysfs has it.
fn disk_of(partition: &str) -> Option<String> {
    let link = std::fs::read_link(format!("/sys/class/block/{partition}")).ok()?;
    let parent = link.parent()?.file_name()?.to_string_lossy().into_owned();
    Some(if parent == "block" { partition.to_string() } else { parent })
}

/// The disk `/boot` is mounted from.
fn boot_disk() -> Option<String> {
    let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
    let device = mounts.lines().find(|l| l.split_whitespace().nth(1) == Some("/boot"))?.split_whitespace().next()?;
    disk_of(device.strip_prefix("/dev/")?)
}

/// The block device whose ext4 volume has this label.
fn find_labelled(label: &str) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir("/sys/class/block").ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    // On the drive the system started from, when it did from one: any stick
    // can call itself this, and an internal disk from another install too.
    let boot = boot_disk();
    names.into_iter().filter(|name| boot.is_none() || disk_of(name) == boot).find(|name| {
        let Ok(mut file) = std::fs::File::open(format!("/dev/{name}")) else { return false };
        let mut sb = [0u8; 136];
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(1024)).is_ok()
            && file.read_exact(&mut sb).is_ok()
            && sb[0x38] == 0x53
            && sb[0x39] == 0xEF
            && sb[0x78..0x88].split(|&b| b == 0).next() == Some(label.as_bytes())
    })
}
