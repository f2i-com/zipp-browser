//! Keyboards, mice, touchpads, tablets and touchscreens, read straight from
//! `/dev/input/event*`, with devices plugged in later picked up through
//! inotify.
//!
//! Keys are keys and relative motion moves the pointer, whatever the device.
//! What absolute X/Y mean depends on the device, which the kernel says
//! (its properties and the keys it has): a touchscreen's place the pointer
//! where the finger is and a touch is a left click; a touchpad's move it the
//! way a mouse does; a tablet's (QEMU's, a pen tablet) place it; anything
//! else's (a gamepad's sticks) are not the pointer's business.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use dpi::PhysicalPosition;
use winit_core::event::{
    ButtonSource, DeviceId, ElementState, KeyEvent, Modifiers, MouseButton, MouseScrollDelta, PointerSource,
    TouchPhase, WindowEvent,
};
use winit_core::keyboard::{ModifiersKeys, ModifiersState};

use crate::keymap;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const REL_HWHEEL: u16 = 0x06;
const REL_WHEEL: u16 = 0x08;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const BTN_MISC: u16 = 0x100;
const BTN_LEFT: u16 = 0x110;
const BTN_TASK: u16 = 0x117;
const BTN_TOOL_PEN: u16 = 0x140;
const BTN_TOOL_FINGER: u16 = 0x145;
const BTN_TOUCH: u16 = 0x14a;
const BTN_STYLUS: u16 = 0x14b;
const KEY_OK: u16 = 0x160;
const KEY_MAX: usize = 0x2ff;
const KEY_CAPSLOCK: u16 = 58;
const INPUT_PROP_POINTER: usize = 0;
const INPUT_PROP_DIRECT: usize = 1;

#[repr(C)]
struct InputEvent {
    time: libc::timeval,
    kind: u16,
    code: u16,
    value: i32,
}

#[repr(C)]
#[derive(Default)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

/// `_IOC(_IOC_READ, 'E', nr, size)`, the same on x86_64 and arm64.
fn eviocg(nr: u64, size: u64) -> libc::c_ulong {
    ((2u64 << 30) | (size << 16) | ((b'E' as u64) << 8) | nr) as libc::c_ulong
}

/// A bitmap the kernel fills in (EVIOCGBIT, EVIOCGPROP, EVIOCGKEY).
fn bits<const N: usize>(fd: RawFd, nr: u64) -> [u8; N] {
    let mut bits = [0u8; N];
    unsafe { libc::ioctl(fd, eviocg(nr, N as u64), bits.as_mut_ptr()) };
    bits
}

fn bit(bits: &[u8], n: usize) -> bool {
    bits.get(n / 8).is_some_and(|b| b & (1 << (n % 8)) != 0)
}

/// What a device's absolute X and Y are.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Absolute {
    /// Where on the screen: a finger, so a touch is a click.
    Touchscreen,
    /// Where on the screen, with buttons of its own: a tablet, or QEMU's.
    Tablet,
    /// Motion, as a mouse's is.
    Touchpad,
    /// Not the pointer at all (a gamepad), or no absolute axes.
    Ignored,
}

struct Device {
    file: File,
    path: PathBuf,
    absolute: Absolute,
    /// The ranges of absolute X and Y, for a device whose they matter.
    abs_range: Option<((i32, i32), (i32, i32))>,
    /// Whether the position comes from the multi-touch axes only.
    mt_only: bool,
    /// Current multi-touch slot (only slot 0 drives the primary pointer).
    mt_slot: i32,
    /// Keys and buttons pressed, by code: what SYN_DROPPED may leave stuck.
    pressed: BTreeSet<u16>,
    /// Events were dropped: ignore the rest of the report, then resync.
    dropped: bool,
    // What has arrived since the last SYN_REPORT.
    rel: (i32, i32),
    wheel: (i32, i32),
    abs: (Option<i32>, Option<i32>),
    touch: Option<bool>,
    /// Mouse buttons queued until SYN_REPORT so pointer motion in the same
    /// report applies before click dispatch.
    buttons: Vec<(MouseButton, ElementState)>,
    /// Touchpad finger lifted during the current SYN_REPORT.
    pad_released: bool,
    /// A touchpad's last position while a finger is down, for its motion.
    pad_last: Option<(i32, i32)>,
    /// Where an absolute device last put the pointer, on the panel.
    panel_pos: (f64, f64),
}

impl Device {
    fn open(path: &Path) -> io::Result<Device> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        let fd = file.as_raw_fd();
        let range = |axis: u16| -> Option<(i32, i32)> {
            let mut info = AbsInfo::default();
            let ok = unsafe { libc::ioctl(fd, eviocg(0x40 + axis as u64, size_of::<AbsInfo>() as u64), &mut info) } >= 0;
            (ok && info.maximum > info.minimum).then_some((info.minimum, info.maximum))
        };
        let abs_bits: [u8; 8] = bits(fd, 0x20 + EV_ABS as u64);
        let keys: [u8; KEY_MAX / 8 + 1] = bits(fd, 0x20 + EV_KEY as u64);
        let props: [u8; 4] = bits(fd, 0x09);
        let (abs_range, mt_only) = match (abs_bits[0] & 0b11 == 0b11, range(ABS_X), range(ABS_Y)) {
            (true, Some(x), Some(y)) => (Some((x, y)), false),
            _ => match (range(ABS_MT_POSITION_X), range(ABS_MT_POSITION_Y)) {
                (Some(x), Some(y)) => (Some((x, y)), true),
                _ => (None, false),
            },
        };
        let key = |code: u16| bit(&keys, code as usize);
        let absolute = if abs_range.is_none() {
            Absolute::Ignored
        } else if bit(&props, INPUT_PROP_DIRECT) {
            Absolute::Touchscreen
        } else if bit(&props, INPUT_PROP_POINTER) || key(BTN_TOOL_FINGER) {
            Absolute::Touchpad
        } else if key(BTN_TOUCH) {
            // An older touchscreen that does not say it is one.
            Absolute::Touchscreen
        } else if key(BTN_LEFT) || key(BTN_TOOL_PEN) || key(BTN_STYLUS) {
            Absolute::Tablet
        } else {
            Absolute::Ignored
        };
        let mut name = [0u8; 128];
        let len = unsafe { libc::ioctl(fd, eviocg(0x06, name.len() as u64), name.as_mut_ptr()) };
        let name = if len > 0 {
            let n = (len as usize).min(name.len());
            String::from_utf8_lossy(&name[..n]).trim_end_matches('\0').to_string()
        } else {
            String::new()
        };
        let kind = match absolute {
            Absolute::Touchscreen => " (touchscreen)",
            Absolute::Tablet => " (absolute)",
            Absolute::Touchpad => " (touchpad)",
            Absolute::Ignored => "",
        };
        eprintln!("[kms] input {}: {name}{kind}", path.display());
        Ok(Device {
            file,
            path: path.to_path_buf(),
            abs_range: abs_range.filter(|_| absolute != Absolute::Ignored),
            absolute,
            mt_only,
            mt_slot: 0,
            pressed: BTreeSet::new(),
            dropped: false,
            rel: (0, 0),
            wheel: (0, 0),
            abs: (None, None),
            touch: None,
            buttons: Vec::new(),
            pad_released: false,
            pad_last: None,
            panel_pos: (0.0, 0.0),
        })
    }
}

pub(crate) struct Input {
    devices: Vec<Device>,
    inotify: Option<OwnedFd>,
    screen: (f64, f64),
    /// The panel's own size, and the quarter turns clockwise the picture is
    /// turned by on it: a touchscreen or a tablet reports where on the
    /// panel, which the picture's point is turned from. (A mouse's or a
    /// touchpad's movement is the user's, whichever way the panel lies.)
    panel: (f64, f64),
    rotation: u8,
    pub(crate) pointer: (f64, f64),
    /// Whether the pointer last moved by touch rather than by a mouse.
    pub(crate) touched: bool,
    mods: ModifiersKeys,
    caps: bool,
}

impl Input {
    /// Input for a screen of `screen` pixels: the pointer stays on it.
    pub(crate) fn new(screen: (u32, u32)) -> Input {
        let inotify = unsafe {
            let fd = libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
            (fd >= 0).then(|| OwnedFd::from_raw_fd(fd))
        };
        let mut input = Input {
            devices: Vec::new(),
            inotify,
            screen: (screen.0 as f64, screen.1 as f64),
            panel: (screen.0 as f64, screen.1 as f64),
            rotation: 0,
            pointer: (screen.0 as f64 / 2.0, screen.1 as f64 / 2.0),
            touched: false,
            mods: ModifiersKeys::empty(),
            caps: false,
        };
        input.watch();
        input.scan();
        input
    }

    /// Watch /dev/input for new devices, and /dev for /dev/input itself:
    /// devtmpfs makes it with the first input device and removes it with the
    /// last, and a watch on a removed directory is silently gone.
    fn watch(&self) {
        if let Some(inotify) = &self.inotify {
            unsafe {
                libc::inotify_add_watch(inotify.as_raw_fd(), c"/dev".as_ptr(), libc::IN_CREATE);
                libc::inotify_add_watch(inotify.as_raw_fd(), c"/dev/input".as_ptr(), libc::IN_CREATE | libc::IN_ATTRIB);
            }
        }
    }

    /// Open every event device not already open.
    fn scan(&mut self) {
        let Ok(dir) = std::fs::read_dir("/dev/input") else { return };
        let mut paths: Vec<PathBuf> = dir
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event")))
            .collect();
        paths.sort();
        for path in paths {
            if self.devices.iter().any(|d| d.path == path) {
                continue;
            }
            if let Ok(device) = Device::open(&path) {
                self.devices.push(device);
            }
        }
    }

    /// Append every input and inotify fd to `poll_fds` without heap reallocating.
    pub(crate) fn push_poll_fds(&self, poll_fds: &mut Vec<libc::pollfd>) {
        for d in &self.devices {
            poll_fds.push(libc::pollfd { fd: d.file.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        }
        if let Some(inotify) = &self.inotify {
            poll_fds.push(libc::pollfd { fd: inotify.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        }
    }

    /// Everything the ready devices have to say, as window events.
    /// `ready` matches the slice appended by [`push_poll_fds`].
    pub(crate) fn read(&mut self, ready: &[libc::pollfd], out: &mut Vec<WindowEvent>) {
        let dev_count = self.devices.len();
        let inotify_ready = self.inotify.is_some()
            && ready.get(dev_count).is_some_and(|p| p.revents & libc::POLLIN != 0);
        if inotify_ready && let Some(inotify) = &self.inotify {
            let mut buf = [0u8; 4096];
            let mut got = false;
            loop {
                let n = unsafe { libc::read(inotify.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
                if n > 0 {
                    got = true;
                } else {
                    break;
                }
            }
            if got {
                self.watch();
                self.scan();
            }
        }

        let mut gone = Vec::new();
        for index in 0..dev_count {
            let revents = ready.get(index).map_or(libc::POLLIN, |p| p.revents);
            if revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) == 0 {
                continue;
            }
            if revents & libc::POLLIN == 0 {
                gone.push(index);
                continue;
            }
            let mut raw = [0u8; size_of::<InputEvent>() * 64];
            loop {
                let n = match self.devices[index].file.read(&mut raw) {
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        gone.push(index);
                        break;
                    }
                };
                if n == 0 {
                    gone.push(index);
                    break;
                }
                for chunk in raw[..n].chunks_exact(size_of::<InputEvent>()) {
                    let event = unsafe { std::ptr::read_unaligned(chunk.as_ptr().cast::<InputEvent>()) };
                    self.event(index, event, out);
                }
            }
        }
        for index in gone.into_iter().rev() {
            eprintln!("[kms] input {} removed", self.devices[index].path.display());
            // Whatever it held down is let go.
            self.release_all(index, out);
            self.devices.remove(index);
        }
    }

    fn event(&mut self, index: usize, event: InputEvent, out: &mut Vec<WindowEvent>) {
        let device = &mut self.devices[index];
        if device.dropped {
            // The kernel's buffer overflowed: the rest of this report is not
            // to be trusted. At its end, what is really down is read back.
            if (event.kind, event.code) == (EV_SYN, SYN_REPORT) {
                device.dropped = false;
                self.resync(index, out);
            }
            return;
        }
        let device_id = Some(DeviceId::from_raw(device.file.as_raw_fd() as i64));
        if event.kind == EV_KEY {
            match event.value {
                0 => device.pressed.remove(&event.code),
                1 => device.pressed.insert(event.code),
                _ => false,
            };
        }
        match (event.kind, event.code) {
            (EV_KEY, BTN_TOUCH) if device.absolute == Absolute::Touchpad => {
                if event.value == 0 {
                    device.pad_last = None;
                    device.pad_released = true;
                }
            }
            (EV_KEY, BTN_TOUCH) if device.abs_range.is_some() => {
                device.touch = Some(event.value != 0);
                if device.absolute == Absolute::Touchscreen {
                    self.touched = true;
                }
            }
            (EV_KEY, BTN_LEFT..=BTN_TASK) => {
                let button = match event.code - BTN_LEFT {
                    0 => MouseButton::Left,
                    1 => MouseButton::Right,
                    2 => MouseButton::Middle,
                    3 | 6 => MouseButton::Back,
                    4 | 5 => MouseButton::Forward,
                    _ => MouseButton::Button6,
                };
                let state = if event.value != 0 { ElementState::Pressed } else { ElementState::Released };
                // Queue until SYN_REPORT so REL/ABS motion in the same packet lands first.
                device.buttons.push((button, state));
            }
            (EV_KEY, code) if !(BTN_MISC..KEY_OK).contains(&code) => self.key(device_id, code, event.value, out),
            (EV_REL, REL_X) => device.rel.0 += event.value,
            (EV_REL, REL_Y) => device.rel.1 += event.value,
            (EV_REL, REL_WHEEL) => device.wheel.1 += event.value,
            (EV_REL, REL_HWHEEL) => device.wheel.0 += event.value,
            (EV_ABS, ABS_X) if !device.mt_only => device.abs.0 = Some(event.value),
            (EV_ABS, ABS_Y) if !device.mt_only => device.abs.1 = Some(event.value),
            (EV_ABS, ABS_MT_SLOT) if device.mt_only => device.mt_slot = event.value,
            (EV_ABS, ABS_MT_POSITION_X) if device.mt_only && device.mt_slot == 0 => device.abs.0 = Some(event.value),
            (EV_ABS, ABS_MT_POSITION_Y) if device.mt_only && device.mt_slot == 0 => device.abs.1 = Some(event.value),
            (EV_SYN, SYN_REPORT) => self.flush(index, device_id, out),
            (EV_SYN, SYN_DROPPED) => {
                device.dropped = true;
                device.rel = (0, 0);
                device.wheel = (0, 0);
                device.abs = (None, None);
                device.touch = None;
                device.buttons.clear();
                device.pad_released = false;
            }
            _ => {}
        }
    }

    /// After dropped events: release whatever is no longer down, by asking
    /// the kernel which keys and buttons are (EVIOCGKEY).
    fn resync(&mut self, index: usize, out: &mut Vec<WindowEvent>) {
        let device = &self.devices[index];
        let down: [u8; KEY_MAX / 8 + 1] = bits(device.file.as_raw_fd(), 0x18);
        let released: Vec<u16> = device.pressed.iter().copied().filter(|&c| !bit(&down, c as usize)).collect();
        for code in released {
            self.event(index, release(code), out);
        }
        self.event(index, report(), out);
        self.devices[index].pad_last = None;
    }

    /// Release everything a device holds down (it went away).
    fn release_all(&mut self, index: usize, out: &mut Vec<WindowEvent>) {
        // Clear dropped so release events are never swallowed on unplug.
        self.devices[index].dropped = false;
        let held: Vec<u16> = self.devices[index].pressed.iter().copied().collect();
        for code in held {
            self.event(index, release(code), out);
        }
        self.event(index, report(), out);
    }

    /// The screen is now this size (the picture's), on a panel of that size
    /// turned that way: the pointer stays on it.
    pub(crate) fn set_screen(&mut self, screen: (u32, u32), panel: (u32, u32), rotation: u8) {
        self.screen = (screen.0 as f64, screen.1 as f64);
        self.panel = (panel.0 as f64, panel.1 as f64);
        self.rotation = rotation % 4;
        self.pointer = (
            self.pointer.0.clamp(0.0, (self.screen.0 - 1.0).max(0.0)),
            self.pointer.1.clamp(0.0, (self.screen.1 - 1.0).max(0.0)),
        );
    }

    /// One SYN_REPORT's worth of motion, wheel, buttons and touch, as events.
    fn flush(&mut self, index: usize, device_id: Option<DeviceId>, out: &mut Vec<WindowEvent>) {
        let (w, h) = self.screen;
        let max_x = (w - 1.0).max(0.0);
        let max_y = (h - 1.0).max(0.0);
        let device = &mut self.devices[index];
        let mut moved = false;
        if device.rel != (0, 0) {
            self.touched = false;
            self.pointer.0 = (self.pointer.0 + device.rel.0 as f64).clamp(0.0, max_x);
            self.pointer.1 = (self.pointer.1 + device.rel.1 as f64).clamp(0.0, max_y);
            device.rel = (0, 0);
            moved = true;
        }
        let pad_released = std::mem::take(&mut device.pad_released);
        if let (Some(((x0, x1), (y0, y1))), (ax, ay)) = (device.abs_range, device.abs) {
            if device.absolute == Absolute::Touchpad {
                if !pad_released {
                    if let Some(last) = device.pad_last {
                        let now = (ax.unwrap_or(last.0), ay.unwrap_or(last.1));
                        // The same scale both ways: a pad's units are square.
                        let scale = w / (x1 - x0) as f64;
                        let (dx, dy) = ((now.0 - last.0) as f64 * scale, (now.1 - last.1) as f64 * scale);
                        if dx != 0.0 || dy != 0.0 {
                            self.touched = false;
                            self.pointer.0 = (self.pointer.0 + dx).clamp(0.0, max_x);
                            self.pointer.1 = (self.pointer.1 + dy).clamp(0.0, max_y);
                            moved = true;
                        }
                        if ax.is_some() || ay.is_some() {
                            device.pad_last = Some(now);
                        }
                    } else if let (Some(x), Some(y)) = (ax, ay) {
                        device.pad_last = Some((x, y));
                    }
                }
            } else {
                // Where on the panel, then where in the picture turned on it.
                let (pw, ph) = self.panel;
                let max_px = (pw - 1.0).max(0.0);
                let max_py = (ph - 1.0).max(0.0);
                if let Some(x) = ax {
                    device.panel_pos.0 = ((x - x0) as f64 * max_px / (x1 - x0) as f64).clamp(0.0, max_px);
                    moved = true;
                }
                if let Some(y) = ay {
                    device.panel_pos.1 = ((y - y0) as f64 * max_py / (y1 - y0) as f64).clamp(0.0, max_py);
                    moved = true;
                }
                if moved {
                    let (px, py) = device.panel_pos;
                    let (x, y) = match self.rotation {
                        1 => (py, max_px - px),
                        2 => (max_px - px, max_py - py),
                        3 => (max_py - py, px),
                        _ => (px, py),
                    };
                    self.pointer = (x.clamp(0.0, max_x), y.clamp(0.0, max_y));
                }
            }
            device.abs = (None, None);
        }
        let wheel = std::mem::take(&mut device.wheel);
        let touch = device.touch.take();
        let buttons = std::mem::take(&mut device.buttons);
        let position = self.position();
        if moved {
            let ev = WindowEvent::PointerMoved { device_id, position, primary: true, source: PointerSource::Mouse };
            // Coalesce consecutive PointerMoved events within one read batch.
            if matches!(out.last(), Some(WindowEvent::PointerMoved { .. })) {
                *out.last_mut().unwrap() = ev;
            } else {
                out.push(ev);
            }
        }
        if wheel != (0, 0) {
            // A notch away from you scrolls up, as winit's positive y does;
            // tilting right reveals the right, which is winit's negative x.
            out.push(WindowEvent::MouseWheel {
                device_id,
                delta: MouseScrollDelta::LineDelta(-wheel.0 as f32, wheel.1 as f32),
                phase: TouchPhase::Moved,
            });
        }
        for (button, state) in buttons {
            out.push(WindowEvent::PointerButton {
                device_id,
                state,
                position,
                primary: true,
                button: ButtonSource::Mouse(button),
            });
        }
        if let Some(down) = touch {
            out.push(WindowEvent::PointerButton {
                device_id,
                state: if down { ElementState::Pressed } else { ElementState::Released },
                position,
                primary: true,
                button: ButtonSource::Mouse(MouseButton::Left),
            });
        }
    }

    fn key(&mut self, device_id: Option<DeviceId>, code: u16, value: i32, out: &mut Vec<WindowEvent>) {
        let state = if value == 0 { ElementState::Released } else { ElementState::Pressed };
        let flag = match code {
            42 => ModifiersKeys::LSHIFT,
            54 => ModifiersKeys::RSHIFT,
            29 => ModifiersKeys::LCONTROL,
            97 => ModifiersKeys::RCONTROL,
            56 => ModifiersKeys::LALT,
            100 => ModifiersKeys::RALT,
            125 => ModifiersKeys::LMETA,
            126 => ModifiersKeys::RMETA,
            _ => ModifiersKeys::empty(),
        };
        if !flag.is_empty() && value != 2 {
            let before = self.mods;
            // Keep modifier set if any other connected keyboard still holds it.
            let still_down = value != 0 || self.devices.iter().any(|d| d.pressed.contains(&code));
            self.mods.set(flag, still_down);
            if self.mods != before {
                out.push(WindowEvent::ModifiersChanged(Modifiers::new(self.modifiers_state(), self.mods)));
            }
        }
        if code == KEY_CAPSLOCK && value == 1 {
            self.caps = !self.caps;
        }

        let mapped = keymap::map(code);
        let mods = self.modifiers_state();
        let shift = mods.shift_key();
        let text = (state.is_pressed() && !mods.control_key() && !mods.alt_key() && !mods.meta_key())
            .then(|| mapped.text(shift, self.caps))
            .flatten()
            .map(|c| c.to_string().into());
        out.push(WindowEvent::KeyboardInput {
            device_id,
            event: KeyEvent {
                physical_key: mapped.physical,
                logical_key: mapped.logical(shift, self.caps),
                text: text.clone(),
                location: mapped.location,
                state,
                repeat: value == 2,
                text_with_all_modifiers: text,
                key_without_modifiers: mapped.logical(false, false),
            },
            is_synthetic: false,
        });
    }

    fn modifiers_state(&self) -> ModifiersState {
        let mut state = ModifiersState::empty();
        let k = self.mods;
        state.set(ModifiersState::SHIFT, k.intersects(ModifiersKeys::LSHIFT | ModifiersKeys::RSHIFT));
        state.set(ModifiersState::CONTROL, k.intersects(ModifiersKeys::LCONTROL | ModifiersKeys::RCONTROL));
        state.set(ModifiersState::ALT, k.intersects(ModifiersKeys::LALT | ModifiersKeys::RALT));
        state.set(ModifiersState::META, k.intersects(ModifiersKeys::LMETA | ModifiersKeys::RMETA));
        state
    }

    fn position(&self) -> PhysicalPosition<f64> {
        PhysicalPosition::new(self.pointer.0, self.pointer.1)
    }
}

/// A key or button's release, as the kernel would send it.
fn release(code: u16) -> InputEvent {
    InputEvent { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, kind: EV_KEY, code, value: 0 }
}

/// The end of a report: what came before it takes effect.
fn report() -> InputEvent {
    InputEvent { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, kind: EV_SYN, code: SYN_REPORT, value: 0 }
}
