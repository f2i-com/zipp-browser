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
use std::os::unix::process::CommandExt;
use std::sync::mpsc::{Sender, channel};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

struct Pty {
    master: OwnedFd,
    pid: i32,
    input: Sender<Vec<u8>>,
}

static PTYS: Mutex<Option<HashMap<u64, Pty>>> = Mutex::new(None);
/// (id, "data" | "exit", text or exit code), for the page.
static EVENTS: Mutex<Vec<(u64, &'static str, String)>> = Mutex::new(Vec::new());
static PAGE: AtomicU64 = AtomicU64::new(0);

/// What has happened on the page's terminals since last asked.
pub(crate) fn take_events() -> Vec<(u64, &'static str, String)> {
    let mut events = std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|e| e.into_inner()));
    // Output of one terminal that came in pieces goes to the page as one.
    let mut merged: Vec<(u64, &'static str, String)> = Vec::with_capacity(events.len());
    for event in events.drain(..) {
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
        // SAFETY: a signal to a process group this module started.
        unsafe { libc::kill(-pty.pid, libc::SIGHUP) };
    }
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

fn push(page: u64, id: u64, kind: &'static str, text: String) {
    if PAGE.load(Ordering::SeqCst) != page {
        return;
    }
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).push((id, kind, text));
    crate::worker::wake();
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
    // The master is the page's alone: not the program's, nor any other's.
    unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };

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
            if let Some(jail) = &jail {
                jail.enter()?;
            }
            Ok(())
        });
    }
    let child = command.spawn().map_err(|e| format!("NotFoundError: {program}: {e}"))?;
    drop(command);
    drop(slave);
    let pid = child.id() as i32;
    let page = PAGE.load(Ordering::SeqCst);

    // What the page types, written on a thread of its own.
    let (tx, rx) = channel::<Vec<u8>>();
    let writer = master.try_clone().map_err(|e| e.to_string())?;
    let _ = std::thread::Builder::new().name("pty-in".into()).spawn(move || {
        let mut file = std::fs::File::from(writer);
        for bytes in rx {
            if file.write_all(&bytes).is_err() {
                break;
            }
        }
    });
    // What the program prints, read on another, and its end.
    let reader = master.try_clone().map_err(|e| e.to_string())?;
    let _ = std::thread::Builder::new().name("pty-out".into()).spawn(move || {
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
                    let whole = match std::str::from_utf8(&carry) {
                        Ok(_) => carry.len(),
                        Err(e) if e.error_len().is_none() => e.valid_up_to(),
                        Err(_) => carry.len(),
                    };
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
    });
    // The child is waited for by the reader; std must not reap it first.
    std::mem::forget(child);
    PTYS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(id, Pty { master, pid, input: tx });
    Ok(pid)
}

/// Bytes for a terminal's program, as typed.
pub(crate) fn write(id: u64, text: &str) -> Result<(), String> {
    let ptys = PTYS.lock().unwrap_or_else(|e| e.into_inner());
    let pty = ptys.as_ref().and_then(|p| p.get(&id)).ok_or("InvalidStateError: that program has ended")?;
    let _ = pty.input.send(text.as_bytes().to_vec());
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
struct JailPaths {
    root: CString,
    proc_: CString,
    dev: CString,
    sys: CString,
    cwd: CString,
    slash: CString,
    procfs: CString,
    host_dev: CString,
    host_sys: CString,
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
        // into /run, which the jail does not see.
        if let Ok(dns) = std::fs::read_to_string("/etc/resolv.conf") {
            let _ = std::fs::write(format!("{root}/etc/resolv.conf"), dns);
        }
        for dir in ["proc", "dev", "sys", "tmp"] {
            let _ = std::fs::create_dir_all(format!("{root}/{dir}"));
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
            host_dev: c("/dev".into())?,
            host_sys: c("/sys".into())?,
            wayland,
            uid: jail.uid,
            gid: jail.gid,
        })
    }

    /// In the child: its own mounts, the root, the user.
    fn enter(&self) -> std::io::Result<()> {
        let check = |rc: libc::c_int| if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) };
        let none = std::ptr::null::<libc::c_char>();
        // SAFETY: system calls on the strings made before the fork.
        unsafe {
            check(libc::unshare(libc::CLONE_NEWNS))?;
            // Nothing mounted here reaches the system's own mounts.
            check(libc::mount(none, self.slash.as_ptr(), none, libc::MS_REC | libc::MS_PRIVATE, std::ptr::null()))?;
            check(libc::mount(self.procfs.as_ptr(), self.proc_.as_ptr(), self.procfs.as_ptr(), libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC, std::ptr::null()))?;
            check(libc::mount(self.host_dev.as_ptr(), self.dev.as_ptr(), none, libc::MS_BIND | libc::MS_REC, std::ptr::null()))?;
            check(libc::mount(self.host_sys.as_ptr(), self.sys.as_ptr(), none, libc::MS_BIND | libc::MS_REC | libc::MS_RDONLY, std::ptr::null()))?;
            if let Some((host, inside)) = &self.wayland {
                check(libc::mount(host.as_ptr(), inside.as_ptr(), none, libc::MS_BIND, std::ptr::null()))?;
            }
            check(libc::chroot(self.root.as_ptr()))?;
            if libc::chdir(self.cwd.as_ptr()) != 0 {
                check(libc::chdir(self.slash.as_ptr()))?;
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
    // SAFETY: valid strings for the call; no options.
    if unsafe { libc::mount(source.as_ptr(), target.as_ptr(), fs.as_ptr(), libc::MS_NOATIME, std::ptr::null()) } != 0 {
        return Err(format!("OperationError: cannot mount the Linux partition: {}", std::io::Error::last_os_error()));
    }
    Ok(ROOT.into())
}

/// The block device whose ext4 volume has this label.
fn find_labelled(label: &str) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir("/sys/class/block").ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names.into_iter().find(|name| {
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
