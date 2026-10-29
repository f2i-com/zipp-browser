//! Keyboards, mice, tablets and touchscreens, read straight from
//! `/dev/input/event*`, with devices plugged in later picked up through
//! inotify.
//!
//! Every device is read the same way, by event type rather than by what the
//! device claims to be: keys are keys, relative motion moves the pointer,
//! absolute motion places it, and a touch is a left click where it lands.

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
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const BTN_MISC: u16 = 0x100;
const BTN_LEFT: u16 = 0x110;
const BTN_TASK: u16 = 0x117;
const BTN_TOUCH: u16 = 0x14a;
const KEY_OK: u16 = 0x160;
const KEY_CAPSLOCK: u16 = 58;

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

struct Device {
    file: File,
    path: PathBuf,
    /// The ranges of absolute X and Y, for a device that has them.
    abs_range: Option<((i32, i32), (i32, i32))>,
    /// Whether the position comes from the multi-touch axes only.
    mt_only: bool,
    // What has arrived since the last SYN_REPORT.
    rel: (i32, i32),
    wheel: (i32, i32),
    abs: (Option<i32>, Option<i32>),
    touch: Option<bool>,
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
        let has_abs = {
            let mut bits = [0u8; 8];
            let ok = unsafe { libc::ioctl(fd, eviocg(0x20 + EV_ABS as u64, bits.len() as u64), bits.as_mut_ptr()) } >= 0;
            ok && bits[0] & 0b11 == 0b11
        };
        let (abs_range, mt_only) = match (has_abs, range(ABS_X), range(ABS_Y)) {
            (true, Some(x), Some(y)) => (Some((x, y)), false),
            _ => match (range(ABS_MT_POSITION_X), range(ABS_MT_POSITION_Y)) {
                (Some(x), Some(y)) => (Some((x, y)), true),
                _ => (None, false),
            },
        };
        let mut name = [0u8; 128];
        let len = unsafe { libc::ioctl(fd, eviocg(0x06, name.len() as u64), name.as_mut_ptr()) };
        let name = if len > 0 {
            String::from_utf8_lossy(&name[..len as usize]).trim_end_matches('\0').to_string()
        } else {
            String::new()
        };
        eprintln!(
            "[kms] input {}: {name}{}",
            path.display(),
            if abs_range.is_some() { " (absolute)" } else { "" }
        );
        Ok(Device { file, path: path.to_path_buf(), abs_range, mt_only, rel: (0, 0), wheel: (0, 0), abs: (None, None), touch: None })
    }
}

pub(crate) struct Input {
    devices: Vec<Device>,
    inotify: Option<OwnedFd>,
    screen: (f64, f64),
    pub(crate) pointer: (f64, f64),
    /// Whether the pointer last moved by touch rather than by a mouse.
    pub(crate) touched: bool,
    mods: ModifiersKeys,
    caps: bool,
}

impl Input {
    pub(crate) fn new(screen: (u32, u32)) -> Input {
        let inotify = unsafe {
            let fd = libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
            (fd >= 0).then(|| {
                libc::inotify_add_watch(fd, c"/dev/input".as_ptr(), libc::IN_CREATE | libc::IN_ATTRIB);
                OwnedFd::from_raw_fd(fd)
            })
        };
        let mut input = Input {
            devices: Vec::new(),
            inotify,
            screen: (screen.0 as f64, screen.1 as f64),
            pointer: (screen.0 as f64 / 2.0, screen.1 as f64 / 2.0),
            touched: false,
            mods: ModifiersKeys::empty(),
            caps: false,
        };
        input.scan();
        input
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

    pub(crate) fn fds(&self) -> Vec<RawFd> {
        self.devices.iter().map(|d| d.file.as_raw_fd()).chain(self.inotify.as_ref().map(|fd| fd.as_raw_fd())).collect()
    }

    /// Everything the devices have to say, as window events.
    pub(crate) fn read(&mut self, out: &mut Vec<WindowEvent>) {
        if let Some(inotify) = &self.inotify {
            let mut buf = [0u8; 4096];
            let n = unsafe { libc::read(inotify.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                self.scan();
            }
        }

        let mut gone = Vec::new();
        for index in 0..self.devices.len() {
            let mut raw = [0u8; size_of::<InputEvent>() * 64];
            loop {
                let n = match self.devices[index].file.read(&mut raw) {
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        gone.push(index);
                        break;
                    }
                };
                if n == 0 {
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
            self.devices.remove(index);
        }
    }

    fn event(&mut self, index: usize, event: InputEvent, out: &mut Vec<WindowEvent>) {
        let device_id = Some(DeviceId::from_raw(self.devices[index].file.as_raw_fd() as i64));
        let device = &mut self.devices[index];
        match (event.kind, event.code) {
            (EV_KEY, BTN_TOUCH) if device.abs_range.is_some() => {
                device.touch = Some(event.value != 0);
                self.touched = true;
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
                out.push(WindowEvent::PointerButton {
                    device_id,
                    state: if event.value != 0 { ElementState::Pressed } else { ElementState::Released },
                    position: self.position(),
                    primary: true,
                    button: ButtonSource::Mouse(button),
                });
            }
            (EV_KEY, code) if code < BTN_MISC || code >= KEY_OK => self.key(device_id, code, event.value, out),
            (EV_REL, REL_X) => device.rel.0 += event.value,
            (EV_REL, REL_Y) => device.rel.1 += event.value,
            (EV_REL, REL_WHEEL) => device.wheel.1 += event.value,
            (EV_REL, REL_HWHEEL) => device.wheel.0 += event.value,
            (EV_ABS, ABS_X) if !device.mt_only => device.abs.0 = Some(event.value),
            (EV_ABS, ABS_Y) if !device.mt_only => device.abs.1 = Some(event.value),
            (EV_ABS, ABS_MT_POSITION_X) if device.mt_only => device.abs.0 = Some(event.value),
            (EV_ABS, ABS_MT_POSITION_Y) if device.mt_only => device.abs.1 = Some(event.value),
            (EV_SYN, SYN_REPORT) => self.flush(index, device_id, out),
            (EV_SYN, SYN_DROPPED) => {
                device.rel = (0, 0);
                device.wheel = (0, 0);
            }
            _ => {}
        }
    }

    /// One SYN_REPORT's worth of motion, wheel and touch, as events.
    fn flush(&mut self, index: usize, device_id: Option<DeviceId>, out: &mut Vec<WindowEvent>) {
        let (w, h) = self.screen;
        let device = &mut self.devices[index];
        let mut moved = false;
        if device.rel != (0, 0) {
            self.touched = false;
            self.pointer.0 = (self.pointer.0 + device.rel.0 as f64).clamp(0.0, w - 1.0);
            self.pointer.1 = (self.pointer.1 + device.rel.1 as f64).clamp(0.0, h - 1.0);
            device.rel = (0, 0);
            moved = true;
        }
        if let (Some(((x0, x1), (y0, y1))), (ax, ay)) = (device.abs_range, device.abs) {
            if let Some(x) = ax {
                self.pointer.0 = ((x - x0) as f64 * (w - 1.0) / (x1 - x0) as f64).clamp(0.0, w - 1.0);
                moved = true;
            }
            if let Some(y) = ay {
                self.pointer.1 = ((y - y0) as f64 * (h - 1.0) / (y1 - y0) as f64).clamp(0.0, h - 1.0);
                moved = true;
            }
            device.abs = (None, None);
        }
        let wheel = std::mem::take(&mut device.wheel);
        let touch = device.touch.take();
        let position = self.position();
        if moved {
            out.push(WindowEvent::PointerMoved { device_id, position, primary: true, source: PointerSource::Mouse });
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
            self.mods.set(flag, value != 0);
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
