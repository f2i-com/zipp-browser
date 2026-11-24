//! VirtualBox: the screen follows the size of the window the machine is
//! shown in, as with VirtualBox's Guest Additions.
//!
//! The host asks for a size with a display change request (VMMDev), which
//! the kernel's vboxguest driver passes on: this says the guest can change
//! its screen (`VMMDEV_GUEST_SUPPORTS_GRAPHICS`, which makes VirtualBox send
//! them as its window is resized), waits for the event on a thread of its
//! own, and hands each size asked for to the event loop.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

const HDR_VERSION: u32 = 0x10001;
const EVENT_DISPLAY_CHANGE_REQUEST: u32 = 1 << 2;
const GUEST_SUPPORTS_GRAPHICS: u32 = 1 << 2;
const VMMDEVREQ_GET_DISPLAY_CHANGE_REQ2: u32 = 54;

/// `struct vbg_ioctl_hdr`.
#[repr(C)]
#[derive(Default)]
struct Header {
    size_in: u32,
    version: u32,
    kind: u32,
    rc: i32,
    size_out: u32,
    reserved: u32,
}

/// The fixed-size requests used here: two words in.
#[repr(C)]
#[derive(Default)]
struct TwoWords {
    hdr: Header,
    a: u32,
    b: u32,
}

/// `VMMDevDisplayChangeRequest2`, its header a `vmmdev_request_header`.
#[repr(C)]
#[derive(Default)]
struct DisplayChange {
    hdr: Header,
    xres: u32,
    yres: u32,
    bpp: u32,
    event_ack: u32,
    display: u32,
}

/// `_IOC(_IOC_READ | _IOC_WRITE, 'V', nr, size)`.
fn iowr(nr: u32, size: usize) -> u32 {
    (3 << 30) | ((size as u32) << 16) | ((b'V' as u32) << 8) | nr
}

/// A fixed-size vboxguest request, `out` bytes coming back after the header.
/// Returns `Ok(())` on success, `Err(Some(errno))` on ioctl failure, or `Err(None)` on VMMDev rc < 0.
fn call(dev: &File, nr: u32, words: &mut TwoWords, out: u32) -> Result<(), Option<i32>> {
    let size = std::mem::size_of::<TwoWords>();
    words.hdr = Header { size_in: size as u32, version: HDR_VERSION, size_out: 24 + out, ..Header::default() };
    // SAFETY: the request is the size the ioctl number says.
    let rc = unsafe { libc::ioctl(dev.as_raw_fd(), iowr(nr, size) as _, words as *mut TwoWords) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().raw_os_error());
    }
    if words.hdr.rc < 0 {
        return Err(None);
    }
    Ok(())
}

/// The size the host wants now (and acknowledging its event): 0 x 0 for none.
fn requested_size(dev: &File, ack: bool) -> Option<(u32, u32)> {
    let size = std::mem::size_of::<DisplayChange>();
    let mut req = DisplayChange {
        hdr: Header { size_in: size as u32, version: HDR_VERSION, kind: VMMDEVREQ_GET_DISPLAY_CHANGE_REQ2, ..Header::default() },
        event_ack: if ack { EVENT_DISPLAY_CHANGE_REQUEST } else { 0 },
        ..DisplayChange::default()
    };
    // VBG_IOCTL_VMMDEV_REQUEST(size): the request is its own header.
    // SAFETY: the request is the size the ioctl number says.
    let rc = unsafe { libc::ioctl(dev.as_raw_fd(), iowr(2, size) as _, &mut req as *mut DisplayChange) };
    (rc == 0 && req.hdr.rc >= 0).then_some((req.xres, req.yres))
}

/// Watch for the sizes the host asks for, if this is a VirtualBox machine
/// with the vboxguest driver: `resize` gets each one and returns `false` when
/// the event loop is gone so this thread exits cleanly.
pub(crate) fn watch(resize: impl Fn(u32, u32) -> bool + Send + 'static, alive: impl Fn() -> bool + Send + 'static) {
    let Ok(dev) = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open("/dev/vboxguest")
    else {
        return;
    };
    // VBG_IOCTL_CHANGE_GUEST_CAPABILITIES: this guest resizes its screen.
    let mut caps = TwoWords { a: GUEST_SUPPORTS_GRAPHICS, ..TwoWords::default() };
    if call(&dev, 14, &mut caps, 8).is_err() {
        eprintln!("[kms] VirtualBox: cannot say the screen resizes: {}", std::io::Error::last_os_error());
        return;
    }
    // VBG_IOCTL_CHANGE_FILTER_MASK: hear of display change requests.
    let mut filter = TwoWords { a: EVENT_DISPLAY_CHANGE_REQUEST, ..TwoWords::default() };
    if call(&dev, 12, &mut filter, 0).is_err() {
        eprintln!("[kms] VirtualBox: cannot hear of size changes: {}", std::io::Error::last_os_error());
        return;
    }
    eprintln!("[kms] VirtualBox: the screen follows the window's size");
    let spawned = std::thread::Builder::new().name("vbox-display".into()).spawn(move || {
        let mut last = (0, 0);
        let mut give = |size: Option<(u32, u32)>| -> bool {
            if let Some((w, h)) = size
                && w > 0
                && h > 0
                && (w, h) != last
            {
                last = (w, h);
                return resize(w, h);
            }
            alive()
        };
        if !give(requested_size(&dev, false)) {
            return;
        }
        while alive() {
            // VBG_IOCTL_WAIT_FOR_EVENTS, a few seconds at a time.
            let mut wait = TwoWords { a: 5000, b: EVENT_DISPLAY_CHANGE_REQUEST, ..TwoWords::default() };
            match call(&dev, 10, &mut wait, 4) {
                Ok(()) => {
                    if wait.a & EVENT_DISPLAY_CHANGE_REQUEST != 0 && !give(requested_size(&dev, true)) {
                        break;
                    }
                }
                Err(Some(libc::ETIMEDOUT | libc::EINTR)) => {}
                Err(_) => std::thread::sleep(std::time::Duration::from_secs(1)),
            }
        }
    });
    if let Err(e) = spawned {
        eprintln!("[kms] VirtualBox: {e}");
    }
}
