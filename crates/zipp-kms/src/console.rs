//! The text console behind the screen. While an application has the screen,
//! the console's keyboard is off and it is in graphics mode: keys reach only
//! the application (not a shell on another console, unseen), and the console
//! draws no cursor and never blanks. Put back as it was found when done.

use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

const KDSETMODE: libc::c_ulong = 0x4B3A;
const KD_TEXT: libc::c_int = 0;
const KD_GRAPHICS: libc::c_int = 1;
const KDGKBMODE: libc::c_ulong = 0x4B44;
const KDSKBMODE: libc::c_ulong = 0x4B45;
const K_OFF: libc::c_int = 4;

pub(crate) struct Console {
    tty: File,
    keyboard: libc::c_int,
}

impl Console {
    /// The foreground console, taken; nothing when there are no consoles.
    pub(crate) fn take() -> Option<Console> {
        let tty = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC)
            .open("/dev/tty0")
            .ok()?;
        let fd = tty.as_raw_fd();
        let mut keyboard: libc::c_int = 0;
        unsafe {
            if libc::ioctl(fd, KDGKBMODE as _, &mut keyboard) < 0 {
                return None;
            }
            if libc::ioctl(fd, KDSKBMODE as _, K_OFF) < 0 {
                return None;
            }
            if libc::ioctl(fd, KDSETMODE as _, KD_GRAPHICS) < 0 {
                libc::ioctl(fd, KDSKBMODE as _, keyboard);
                return None;
            }
        }
        Some(Console { tty, keyboard })
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        let fd = self.tty.as_raw_fd();
        unsafe {
            libc::ioctl(fd, KDSETMODE as _, KD_TEXT);
            libc::ioctl(fd, KDSKBMODE as _, self.keyboard);
        }
    }
}
