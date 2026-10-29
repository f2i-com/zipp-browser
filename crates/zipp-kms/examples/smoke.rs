//! `smoke [frames]`: draw a moving pattern through softbuffer's KMS backend
//! on zipp-kms's event loop, the way zipp-browser presents, and report the
//! frame rate. Checks a kernel's DRM setup without the browser in the loop.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("smoke runs on Linux's DRM/KMS only");
}

#[cfg(target_os = "linux")]
fn main() {
    linux::main();
}

#[cfg(target_os = "linux")]
mod linux {
    use std::num::NonZeroU32;
    use std::sync::Arc;
    use std::time::Instant;

    use winit_core::application::ApplicationHandler;
    use winit_core::event::{ElementState, WindowEvent};
    use winit_core::event_loop::ActiveEventLoop;
    use winit_core::window::{Window, WindowAttributes, WindowId};

    struct App {
        frames: u32,
        limit: u32,
        started: Instant,
        window: Option<Arc<dyn Window>>,
        surface: Option<softbuffer::Surface<Arc<dyn Window>, Arc<dyn Window>>>,
    }

    impl ApplicationHandler for App {
        fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
            let window: Arc<dyn Window> = Arc::from(event_loop.create_window(WindowAttributes::default()).unwrap());
            let context = softbuffer::Context::new(window.clone()).unwrap();
            let mut surface = softbuffer::Surface::new(&context, window.clone()).unwrap();
            let size = window.surface_size();
            surface.resize(NonZeroU32::new(size.width).unwrap(), NonZeroU32::new(size.height).unwrap()).unwrap();
            window.request_redraw();
            self.window = Some(window);
            self.surface = Some(surface);
            self.started = Instant::now();
        }

        fn window_event(&mut self, event_loop: &dyn ActiveEventLoop, _id: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::RedrawRequested => {
                    let window = self.window.as_ref().unwrap();
                    let size = window.surface_size();
                    let surface = self.surface.as_mut().unwrap();
                    let mut buffer = surface.buffer_mut().unwrap();
                    let f = self.frames;
                    let bar = (f * 8) % size.width;
                    for y in 0..size.height {
                        for x in 0..size.width {
                            let px = if x.abs_diff(bar) < 24 {
                                0x00ff_ffff
                            } else {
                                let r = (x * 255 / size.width) & 0xff;
                                let g = (y * 255 / size.height) & 0xff;
                                let b = (f * 3) & 0xff;
                                (r << 16) | (g << 8) | b
                            };
                            buffer[(y * size.width + x) as usize] = px;
                        }
                    }
                    if let Err(e) = buffer.present() {
                        eprintln!("[smoke] present failed at frame {f}: {e}");
                    }
                    self.frames += 1;
                    if self.frames % 60 == 0 {
                        let fps = self.frames as f64 / self.started.elapsed().as_secs_f64();
                        eprintln!("[smoke] {} frames, {fps:.1} fps", self.frames);
                    }
                    if self.frames >= self.limit {
                        eprintln!("[smoke] done: {} frames at {}x{}", self.frames, size.width, size.height);
                        event_loop.exit();
                    } else {
                        window.request_redraw();
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                    eprintln!("[smoke] key {:?} text {:?}", event.logical_key, event.text);
                }
                WindowEvent::PointerButton { state, position, .. } => {
                    eprintln!("[smoke] button {state:?} at {:.0},{:.0}", position.x, position.y);
                }
                _ => {}
            }
        }
    }

    pub fn main() {
        let limit = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(300);
        let event_loop = zipp_kms::EventLoop::new().expect("open the screen");
        eprintln!("[smoke] screen {:?}", event_loop.size());
        let app = App { frames: 0, limit, started: Instant::now(), window: None, surface: None };
        event_loop.run_app(app).unwrap();
    }
}
