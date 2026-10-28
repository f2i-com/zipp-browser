//! WebGL 1 for `<canvas>`, on desktop OpenGL through WGL (Windows).
//!
//! The page side is `webgl_shim.js`: a `WebGLRenderingContext` that records
//! its calls into a batch of `[op, ...args]` and sends it here with the
//! `gl` host call, bytes beside it in `__zippHostBytes`. A GL thread replays
//! batches: one OpenGL context per canvas, on a hidden window, each drawing
//! into an offscreen framebuffer that stands in for the canvas's default one
//! (`bindFramebuffer(null)` binds it). A `present` at the end of a frame reads
//! that framebuffer back; the frame goes to [`take_frames`], and the page's
//! canvas element gets it as its image (`apply_canvas_frames` in main.rs).
//!
//! The GL thread keeps OpenGL off the window's thread, which never has a GL
//! context of its own. Each batch is synchronous: the page waits for the
//! answer to its last command (a query, or nothing).
//!
//! Shaders are GLSL ES 1.00. They compile as GLSL 1.20, which every desktop
//! driver takes: `precision` statements go and `lowp`/`mediump`/`highp`
//! become nothing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CString, c_void};
use std::sync::mpsc::{self, Receiver, Sender};

use glow::HasContext;
use serde_json::Value as Json;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetDC, HDC};
use windows::Win32::Graphics::OpenGL::{
    ChoosePixelFormat, HGLRC, PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW, PFD_MAIN_PLANE, PFD_SUPPORT_OPENGL,
    PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR, SetPixelFormat, wglCreateContext, wglGetProcAddress, wglMakeCurrent,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryA};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{PCSTR, s, w};
use zipp_vm::embed::HostCtx;

/// A frame read back from a canvas: its element's `data-flid`, size and RGBA.
pub(crate) struct Frame {
    pub flid: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

struct Batch {
    context: u32,
    cmds: String,
    payload: Vec<u8>,
    reply: Sender<Reply>,
}

#[derive(Default)]
struct Reply {
    /// The answer to the batch's last command, as JSON ("" for none).
    result: String,
    error: Option<String>,
    /// `readPixels`' bytes, for `__zippHostBytes`.
    bytes: Option<Vec<u8>>,
    frame: Option<Frame>,
}

thread_local! {
    static GL_THREAD: RefCell<Option<Sender<Batch>>> = const { RefCell::new(None) };
    static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

/// The frames presented since the last call, for the canvases to show.
pub(crate) fn take_frames() -> Vec<Frame> {
    FRAMES.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

/// `__zippHostCall("gl", context, batch, payloadLength)`.
pub(crate) fn host_call(ctx: &mut dyn HostCtx, args: &[String]) -> Result<String, String> {
    let context: u32 = args.first().and_then(|a| a.parse().ok()).ok_or("Error: gl: no context")?;
    let cmds = args.get(1).cloned().unwrap_or_default();
    let len: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(0);
    let payload = if len > 0 {
        let (addr, count, kind) = ctx.typed_array_region("__zippHostBytes")?;
        if kind != 1 || count < len {
            return Err("Error: gl: __zippHostBytes is not the batch's bytes".into());
        }
        // SAFETY: the engine pinned `count` bytes at `addr` for this call.
        unsafe { std::slice::from_raw_parts(addr as *const u8, len) }.to_vec()
    } else {
        Vec::new()
    };
    let sender = GL_THREAD.with(|t| {
        t.borrow_mut()
            .get_or_insert_with(|| {
                let (tx, rx) = mpsc::channel();
                std::thread::Builder::new()
                    .name("zipp-webgl".into())
                    .spawn(move || gl_thread(rx))
                    .expect("WebGL thread");
                tx
            })
            .clone()
    });
    let (reply_tx, reply_rx) = mpsc::channel();
    sender
        .send(Batch { context, cmds, payload, reply: reply_tx })
        .map_err(|_| "Error: WebGL is unavailable".to_string())?;
    let reply = reply_rx.recv().map_err(|_| "Error: WebGL stopped".to_string())?;
    if let Some(error) = reply.error {
        eprintln!("[WEBGL] {error}");
    }
    if let Some(bytes) = reply.bytes {
        let (addr, count, kind) = ctx.typed_array_region("__zippHostBytes")?;
        if kind == 1 {
            // SAFETY: as above; never more than the array holds.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len().min(count)) };
        }
    }
    if let Some(frame) = reply.frame {
        FRAMES.with(|f| {
            let mut frames = f.borrow_mut();
            frames.retain(|old| old.flid != frame.flid);
            frames.push(frame);
        });
    }
    Ok(reply.result)
}

fn gl_thread(batches: Receiver<Batch>) {
    let mut host = match Wgl::new() {
        Ok(host) => Some(host),
        Err(e) => {
            eprintln!("[WEBGL] no OpenGL: {e}");
            None
        }
    };
    for batch in batches {
        let reply = match host.as_mut() {
            Some(host) => host.run(batch.context, &batch.cmds, &batch.payload),
            None => Reply { error: Some("OpenGL is unavailable".into()), ..Default::default() },
        };
        let _ = batch.reply.send(reply);
    }
}

/// The hidden window's procedure: it handles nothing itself.
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// The hidden window every context draws through, and the contexts.
struct Wgl {
    dc: HDC,
    opengl32: HMODULE,
    current: Option<u32>,
    contexts: HashMap<u32, Context>,
}

/// One canvas's GL context and the WebGL objects in it, by the page's numbers.
struct Context {
    hglrc: HGLRC,
    gl: glow::Context,
    flid: u32,
    width: i32,
    height: i32,
    fbo: glow::Framebuffer,
    color: glow::Renderbuffer,
    depth: glow::Renderbuffer,
    buffers: HashMap<u32, glow::Buffer>,
    textures: HashMap<u32, glow::Texture>,
    shaders: HashMap<u32, glow::Shader>,
    programs: HashMap<u32, glow::Program>,
    framebuffers: HashMap<u32, glow::Framebuffer>,
    renderbuffers: HashMap<u32, glow::Renderbuffer>,
    vaos: HashMap<u32, glow::VertexArray>,
    uniforms: HashMap<(u32, String), Option<glow::UniformLocation>>,
    flip_y: bool,
    premultiply: bool,
}

impl Wgl {
    fn new() -> Result<Self, String> {
        unsafe {
            let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: w!("ZippWebGL"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd: HWND = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("ZippWebGL"),
                w!("Zipp WebGL"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                16,
                16,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            let dc = GetDC(Some(hwnd));
            let pfd = PIXELFORMATDESCRIPTOR {
                nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                nVersion: 1,
                dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                iPixelType: PFD_TYPE_RGBA,
                cColorBits: 32,
                cDepthBits: 24,
                cStencilBits: 8,
                iLayerType: PFD_MAIN_PLANE.0 as u8,
                ..Default::default()
            };
            let format = ChoosePixelFormat(dc, &pfd);
            if format == 0 {
                return Err("no pixel format".into());
            }
            SetPixelFormat(dc, format, &pfd).map_err(|e| e.to_string())?;
            let opengl32 = LoadLibraryA(s!("opengl32.dll")).map_err(|e| e.to_string())?;
            Ok(Wgl { dc, opengl32, current: None, contexts: HashMap::new() })
        }
    }

    fn proc_address(opengl32: HMODULE, name: &str) -> *const c_void {
        let Ok(c) = CString::new(name) else {
            return std::ptr::null();
        };
        unsafe {
            let p = wglGetProcAddress(PCSTR(c.as_ptr() as *const u8)).map(|f| f as usize).unwrap_or(0);
            // wglGetProcAddress answers 1, 2, 3 or -1 for "not mine": the
            // OpenGL 1.1 entry points live in opengl32.dll itself.
            if p > 3 && p != usize::MAX {
                return p as *const c_void;
            }
            GetProcAddress(opengl32, PCSTR(c.as_ptr() as *const u8)).map(|f| f as usize).unwrap_or(0) as *const c_void
        }
    }

    fn make_current(&mut self, context: u32) -> bool {
        if self.current == Some(context) {
            return true;
        }
        let Some(ctx) = self.contexts.get(&context) else {
            return false;
        };
        let ok = unsafe { wglMakeCurrent(self.dc, ctx.hglrc) }.is_ok();
        self.current = ok.then_some(context);
        ok
    }

    fn create(&mut self, context: u32, flid: u32, width: i32, height: i32) -> Result<(), String> {
        unsafe {
            let hglrc = wglCreateContext(self.dc).map_err(|e| e.to_string())?;
            wglMakeCurrent(self.dc, hglrc).map_err(|e| e.to_string())?;
            self.current = Some(context);
            let opengl32 = self.opengl32;
            let gl = glow::Context::from_loader_function(|name| Self::proc_address(opengl32, name));
            let fbo = gl.create_framebuffer()?;
            let color = gl.create_renderbuffer()?;
            let depth = gl.create_renderbuffer()?;
            let mut ctx = Context {
                hglrc,
                gl,
                flid,
                width: 0,
                height: 0,
                fbo,
                color,
                depth,
                buffers: HashMap::new(),
                textures: HashMap::new(),
                shaders: HashMap::new(),
                programs: HashMap::new(),
                framebuffers: HashMap::new(),
                renderbuffers: HashMap::new(),
                vaos: HashMap::new(),
                uniforms: HashMap::new(),
                flip_y: false,
                premultiply: false,
            };
            ctx.resize(width, height);
            ctx.gl.viewport(0, 0, ctx.width, ctx.height);
            self.contexts.insert(context, ctx);
        }
        Ok(())
    }

    fn run(&mut self, context: u32, cmds: &str, payload: &[u8]) -> Reply {
        let mut reply = Reply::default();
        let cmds: Vec<Json> = match serde_json::from_str(cmds) {
            Ok(Json::Array(c)) => c,
            _ => {
                reply.error = Some("malformed batch".into());
                return reply;
            }
        };
        for cmd in &cmds {
            let Some(op) = cmd.get(0).and_then(Json::as_str) else {
                continue;
            };
            if op == "create" {
                let n = |i: usize| cmd.get(i).and_then(Json::as_f64).unwrap_or(0.0);
                if let Err(e) = self.create(context, n(1) as u32, n(2) as i32, n(3) as i32) {
                    reply.error = Some(format!("cannot create a context: {e}"));
                }
                continue;
            }
            if !self.make_current(context) {
                reply.error = Some("no such context".into());
                return reply;
            }
            let ctx = self.contexts.get_mut(&context).unwrap();
            match unsafe { ctx.exec(op, cmd, payload, &mut reply) } {
                Ok(result) => reply.result = result,
                Err(e) => reply.error = Some(format!("{op}: {e}")),
            }
        }
        reply
    }
}

/// GLSL ES 1.00 as GLSL 1.20 (see the module docs).
fn translate_shader(source: &str) -> String {
    let mut body = String::with_capacity(source.len() + 64);
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#version") {
            continue;
        }
        // Built into desktop GLSL fragment shaders.
        if trimmed.starts_with("#extension") && trimmed.contains("GL_OES_standard_derivatives") {
            continue;
        }
        let mut line = line.to_string();
        // `precision mediump float;`, anywhere on the line.
        while let Some(at) = line.find("precision ") {
            match line[at..].find(';') {
                Some(end) => line.replace_range(at..at + end + 1, ""),
                None => break,
            }
        }
        body.push_str(&line);
        body.push('\n');
    }
    format!("#version 120\n#define lowp\n#define mediump\n#define highp\n{body}")
}

/// Turn a bottom-up RGBA image upside down (GL reads bottom row first).
fn flip_rows(rgba: &mut [u8], width: usize, height: usize, bpp: usize) {
    let row = width * bpp;
    for y in 0..height / 2 {
        let (top, bottom) = rgba.split_at_mut((height - 1 - y) * row);
        top[y * row..y * row + row].swap_with_slice(&mut bottom[..row]);
    }
}

fn bytes_per_pixel(format: u32, ty: u32) -> usize {
    let channels = match format {
        glow::RGBA => 4,
        glow::RGB => 3,
        glow::LUMINANCE_ALPHA => 2,
        _ => 1,
    };
    match ty {
        glow::FLOAT => channels * 4,
        glow::UNSIGNED_SHORT_5_6_5 | glow::UNSIGNED_SHORT_4_4_4_4 | glow::UNSIGNED_SHORT_5_5_5_1 => 2,
        _ => channels,
    }
}

impl Context {
    unsafe fn resize(&mut self, width: i32, height: i32) {
        let (width, height) = (width.max(1), height.max(1));
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        unsafe {
            let gl = &self.gl;
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(self.color));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, width, height);
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(self.depth));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH24_STENCIL8, width, height);
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::RENDERBUFFER, Some(self.color));
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_STENCIL_ATTACHMENT, glow::RENDERBUFFER, Some(self.depth));
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT);
        }
    }

    fn uniform_location(&mut self, program: u32, name: &str) -> Option<glow::UniformLocation> {
        let key = (program, name.to_string());
        if let Some(found) = self.uniforms.get(&key) {
            return found.clone();
        }
        let p = *self.programs.get(&program)?;
        let found = unsafe { self.gl.get_uniform_location(p, name) };
        self.uniforms.insert(key, found.clone());
        found
    }

    /// Upload RGBA8 pixels decoded from an image, honouring the unpack flags.
    unsafe fn upload_rgba(&mut self, target: u32, level: i32, internal: i32, width: i32, height: i32, mut rgba: Vec<u8>) {
        if self.flip_y {
            flip_rows(&mut rgba, width as usize, height as usize, 4);
        }
        if self.premultiply {
            for px in rgba.chunks_exact_mut(4) {
                let a = px[3] as u32;
                for c in &mut px[..3] {
                    *c = ((*c as u32 * a + 127) / 255) as u8;
                }
            }
        }
        unsafe {
            self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            self.gl.tex_image_2d(
                target,
                level,
                internal,
                width,
                height,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&rgba)),
            );
        }
    }

    /// Run one command; its answer as JSON, or "".
    unsafe fn exec(&mut self, op: &str, cmd: &Json, payload: &[u8], reply: &mut Reply) -> Result<String, String> {
        let num = |i: usize| cmd.get(i).and_then(Json::as_f64).unwrap_or(0.0);
        let u = |i: usize| num(i) as u32;
        let int = |i: usize| num(i) as i32;
        let f = |i: usize| num(i) as f32;
        let flag = |i: usize| cmd.get(i).and_then(Json::as_bool).unwrap_or(false);
        let text = |i: usize| cmd.get(i).and_then(Json::as_str).unwrap_or("").to_string();
        let bytes = |off: usize, len: usize| -> Result<&[u8], String> {
            payload.get(off..off + len).ok_or_else(|| "bytes out of range".to_string())
        };
        let mut out = String::new();
        unsafe {
            match op {
                "resize" => {
                    let (w, h) = (int(1), int(2));
                    self.resize(w, h);
                }
                "createBuffer" => { let b = self.gl.create_buffer()?; self.buffers.insert(u(1), b); }
                "deleteBuffer" => { if let Some(b) = self.buffers.remove(&u(1)) { self.gl.delete_buffer(b); } }
                "bindBuffer" => self.gl.bind_buffer(u(1), self.buffers.get(&u(2)).copied()),
                "bufferData" => self.gl.buffer_data_u8_slice(u(1), bytes(u(2) as usize, u(3) as usize)?, u(4)),
                "bufferSize" => self.gl.buffer_data_size(u(1), int(2), u(3)),
                "bufferSubData" => self.gl.buffer_sub_data_u8_slice(u(1), int(2), bytes(u(3) as usize, u(4) as usize)?),
                "createShader" => { let s = self.gl.create_shader(u(2))?; self.shaders.insert(u(1), s); }
                "deleteShader" => { if let Some(s) = self.shaders.remove(&u(1)) { self.gl.delete_shader(s); } }
                "shaderSource" => {
                    if let Some(&s) = self.shaders.get(&u(1)) {
                        self.gl.shader_source(s, &translate_shader(&text(2)));
                    }
                }
                "compileShader" => { if let Some(&s) = self.shaders.get(&u(1)) { self.gl.compile_shader(s); } }
                "getShaderParameter" => {
                    let s = self.shaders.get(&u(1)).copied();
                    out = match (s, u(2)) {
                        (Some(s), glow::COMPILE_STATUS) => self.gl.get_shader_compile_status(s).to_string(),
                        (_, glow::DELETE_STATUS) => "false".into(),
                        _ => "null".into(),
                    };
                }
                "getShaderInfoLog" => {
                    let log = self.shaders.get(&u(1)).map(|&s| self.gl.get_shader_info_log(s)).unwrap_or_default();
                    out = Json::String(log).to_string();
                }
                "createProgram" => { let p = self.gl.create_program()?; self.programs.insert(u(1), p); }
                "deleteProgram" => { if let Some(p) = self.programs.remove(&u(1)) { self.gl.delete_program(p); } }
                "attachShader" | "detachShader" => {
                    if let (Some(&p), Some(&s)) = (self.programs.get(&u(1)), self.shaders.get(&u(2))) {
                        if op == "attachShader" { self.gl.attach_shader(p, s) } else { self.gl.detach_shader(p, s) }
                    }
                }
                "linkProgram" => {
                    if let Some(&p) = self.programs.get(&u(1)) {
                        self.gl.link_program(p);
                        let program = u(1);
                        self.uniforms.retain(|(prog, _), _| *prog != program);
                    }
                }
                "validateProgram" => {}
                "getProgramParameter" => {
                    let p = self.programs.get(&u(1)).copied();
                    out = match (p, u(2)) {
                        (Some(p), glow::LINK_STATUS) => self.gl.get_program_link_status(p).to_string(),
                        (Some(p), glow::ACTIVE_ATTRIBUTES) => self.gl.get_active_attributes(p).to_string(),
                        (Some(p), glow::ACTIVE_UNIFORMS) => self.gl.get_active_uniforms(p).to_string(),
                        (Some(_), glow::VALIDATE_STATUS) => "true".into(),
                        (_, glow::DELETE_STATUS) => "false".into(),
                        (Some(_), glow::ATTACHED_SHADERS) => "2".into(),
                        _ => "null".into(),
                    };
                }
                "getProgramInfoLog" => {
                    let log = self.programs.get(&u(1)).map(|&p| self.gl.get_program_info_log(p)).unwrap_or_default();
                    out = Json::String(log).to_string();
                }
                "useProgram" => self.gl.use_program(self.programs.get(&u(1)).copied()),
                "getAttribLocation" => {
                    let at = self.programs.get(&u(1)).and_then(|&p| self.gl.get_attrib_location(p, &text(2)));
                    out = at.map(|a| a as i64).unwrap_or(-1).to_string();
                }
                "bindAttribLocation" => {
                    if let Some(&p) = self.programs.get(&u(1)) {
                        self.gl.bind_attrib_location(p, u(2), &text(3));
                    }
                }
                "getUniformLocation" => {
                    let (program, name) = (u(1), text(2));
                    out = self.uniform_location(program, &name).is_some().to_string();
                }
                "getActiveAttrib" | "getActiveUniform" => {
                    let p = self.programs.get(&u(1)).copied();
                    let info = p.and_then(|p| {
                        if op == "getActiveAttrib" {
                            self.gl.get_active_attribute(p, u(2)).map(|a| (a.name, a.size, a.atype))
                        } else {
                            self.gl.get_active_uniform(p, u(2)).map(|a| (a.name, a.size, a.utype))
                        }
                    });
                    out = match info {
                        Some((name, size, ty)) => serde_json::json!([name, size, ty]).to_string(),
                        None => "null".into(),
                    };
                }
                "enableVertexAttribArray" => self.gl.enable_vertex_attrib_array(u(1)),
                "disableVertexAttribArray" => self.gl.disable_vertex_attrib_array(u(1)),
                "vertexAttribPointer" => self.gl.vertex_attrib_pointer_f32(u(1), int(2), u(3), flag(4), int(5), int(6)),
                "vertexAttrib" => self.gl.vertex_attrib_4_f32(u(1), f(2), f(3), f(4), f(5)),
                "vertexAttribDivisor" => self.gl.vertex_attrib_divisor(u(1), u(2)),
                "uniform" => {
                    let (program, name, kind) = (u(1), text(2), text(3));
                    let values: Vec<f64> = cmd.get(4).and_then(Json::as_array)
                        .map(|a| a.iter().filter_map(Json::as_f64).collect()).unwrap_or_default();
                    if let Some(loc) = self.uniform_location(program, &name) {
                                    let fs: Vec<f32> = values.iter().map(|&v| v as f32).collect();
                        let is: Vec<i32> = values.iter().map(|&v| v as i32).collect();
                        let loc = Some(&loc);
                        match kind.as_str() {
                            "1f" => self.gl.uniform_1_f32_slice(loc, &fs),
                            "2f" => self.gl.uniform_2_f32_slice(loc, &fs),
                            "3f" => self.gl.uniform_3_f32_slice(loc, &fs),
                            "4f" => self.gl.uniform_4_f32_slice(loc, &fs),
                            "1i" => self.gl.uniform_1_i32_slice(loc, &is),
                            "2i" => self.gl.uniform_2_i32_slice(loc, &is),
                            "3i" => self.gl.uniform_3_i32_slice(loc, &is),
                            "4i" => self.gl.uniform_4_i32_slice(loc, &is),
                            "m2" => self.gl.uniform_matrix_2_f32_slice(loc, false, &fs),
                            "m3" => self.gl.uniform_matrix_3_f32_slice(loc, false, &fs),
                            "m4" => self.gl.uniform_matrix_4_f32_slice(loc, false, &fs),
                            _ => {}
                        }
                    }
                }
                "createTexture" => { let t = self.gl.create_texture()?; self.textures.insert(u(1), t); }
                "deleteTexture" => { if let Some(t) = self.textures.remove(&u(1)) { self.gl.delete_texture(t); } }
                "bindTexture" => self.gl.bind_texture(u(1), self.textures.get(&u(2)).copied()),
                "activeTexture" => self.gl.active_texture(u(1)),
                "texParameteri" => self.gl.tex_parameter_i32(u(1), u(2), int(3)),
                "pixelStorei" => match u(1) {
                    0x9240 => self.flip_y = num(2) != 0.0,
                    0x9241 => self.premultiply = num(2) != 0.0,
                    glow::UNPACK_ALIGNMENT | glow::PACK_ALIGNMENT => self.gl.pixel_store_i32(u(1), int(2)),
                    _ => {}
                },
                "generateMipmap" => self.gl.generate_mipmap(u(1)),
                "texImage2D" => {
                    let (target, level, internal, w, h, format, ty) = (u(1), int(2), int(3), int(4), int(5), u(6), u(7));
                    let at = num(8);
                    if at < 0.0 {
                        self.gl.tex_image_2d(target, level, internal, w, h, 0, format, ty, glow::PixelUnpackData::Slice(None));
                    } else {
                        let mut data = bytes(at as usize, u(9) as usize)?.to_vec();
                        if self.flip_y {
                            flip_rows(&mut data, w as usize, h as usize, bytes_per_pixel(format, ty));
                        }
                        self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                        self.gl.tex_image_2d(target, level, internal, w, h, 0, format, ty, glow::PixelUnpackData::Slice(Some(&data)));
                    }
                }
                "texImage2DUrl" => {
                    let (target, level, internal, url) = (u(1), int(2), int(3), text(6));
                    match crate::fetch_bytes_for_page(&url).map_err(|e| e.to_string()).and_then(|(_, raw)| {
                        image::load_from_memory(&raw).map_err(|e| e.to_string())
                    }) {
                        Ok(img) => {
                            let rgba = img.to_rgba8();
                            let (w, h) = (rgba.width() as i32, rgba.height() as i32);
                            self.upload_rgba(target, level, internal, w, h, rgba.into_raw());
                        }
                        Err(e) => reply.error = Some(format!("texture {url}: {e}")),
                    }
                }
                "texSubImage2D" => {
                    let (target, level, x, y, w, h, format, ty) = (u(1), int(2), int(3), int(4), int(5), int(6), u(7), u(8));
                    let mut data = bytes(u(9) as usize, u(10) as usize)?.to_vec();
                    if self.flip_y {
                        flip_rows(&mut data, w as usize, h as usize, bytes_per_pixel(format, ty));
                    }
                    self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                    self.gl.tex_sub_image_2d(target, level, x, y, w, h, format, ty, glow::PixelUnpackData::Slice(Some(&data)));
                }
                "createFramebuffer" => { let fb = self.gl.create_framebuffer()?; self.framebuffers.insert(u(1), fb); }
                "deleteFramebuffer" => { if let Some(fb) = self.framebuffers.remove(&u(1)) { self.gl.delete_framebuffer(fb); } }
                "bindFramebuffer" => {
                    // Null is the canvas: its offscreen framebuffer.
                    let fb = if u(2) == 0 { Some(self.fbo) } else { self.framebuffers.get(&u(2)).copied() };
                    self.gl.bind_framebuffer(u(1), fb);
                }
                "framebufferTexture2D" => {
                    self.gl.framebuffer_texture_2d(u(1), u(2), u(3), self.textures.get(&u(4)).copied(), int(5));
                }
                "framebufferRenderbuffer" => {
                    self.gl.framebuffer_renderbuffer(u(1), u(2), u(3), self.renderbuffers.get(&u(4)).copied());
                }
                "checkFramebufferStatus" => out = self.gl.check_framebuffer_status(u(1)).to_string(),
                "createRenderbuffer" => { let rb = self.gl.create_renderbuffer()?; self.renderbuffers.insert(u(1), rb); }
                "deleteRenderbuffer" => { if let Some(rb) = self.renderbuffers.remove(&u(1)) { self.gl.delete_renderbuffer(rb); } }
                "bindRenderbuffer" => self.gl.bind_renderbuffer(u(1), self.renderbuffers.get(&u(2)).copied()),
                "renderbufferStorage" => {
                    // WebGL's DEPTH_STENCIL is a sized format on the desktop.
                    let format = if u(2) == 0x84f9 { glow::DEPTH24_STENCIL8 } else { u(2) };
                    self.gl.renderbuffer_storage(u(1), format, int(3), int(4));
                }
                "createVertexArray" => { let v = self.gl.create_vertex_array()?; self.vaos.insert(u(1), v); }
                "deleteVertexArray" => { if let Some(v) = self.vaos.remove(&u(1)) { self.gl.delete_vertex_array(v); } }
                "bindVertexArray" => self.gl.bind_vertex_array(self.vaos.get(&u(1)).copied()),
                "enable" => self.gl.enable(u(1)),
                "disable" => self.gl.disable(u(1)),
                "viewport" => self.gl.viewport(int(1), int(2), int(3), int(4)),
                "scissor" => self.gl.scissor(int(1), int(2), int(3), int(4)),
                "clearColor" => self.gl.clear_color(f(1), f(2), f(3), f(4)),
                "clearDepth" => self.gl.clear_depth_f64(num(1)),
                "clearStencil" => self.gl.clear_stencil(int(1)),
                "clear" => self.gl.clear(u(1)),
                "colorMask" => self.gl.color_mask(flag(1), flag(2), flag(3), flag(4)),
                "depthMask" => self.gl.depth_mask(flag(1)),
                "depthFunc" => self.gl.depth_func(u(1)),
                "depthRange" => self.gl.depth_range_f64(num(1), num(2)),
                "blendFuncSeparate" => self.gl.blend_func_separate(u(1), u(2), u(3), u(4)),
                "blendEquationSeparate" => self.gl.blend_equation_separate(u(1), u(2)),
                "blendColor" => self.gl.blend_color(f(1), f(2), f(3), f(4)),
                "cullFace" => self.gl.cull_face(u(1)),
                "frontFace" => self.gl.front_face(u(1)),
                "lineWidth" => self.gl.line_width(f(1)),
                "polygonOffset" => self.gl.polygon_offset(f(1), f(2)),
                "stencilFuncSeparate" => self.gl.stencil_func_separate(u(1), u(2), int(3), u(4)),
                "stencilOpSeparate" => self.gl.stencil_op_separate(u(1), u(2), u(3), u(4)),
                "stencilMaskSeparate" => self.gl.stencil_mask_separate(u(1), u(2)),
                "drawArrays" => self.gl.draw_arrays(u(1), int(2), int(3)),
                "drawElements" => self.gl.draw_elements(u(1), int(2), u(3), int(4)),
                "drawArraysInstanced" => self.gl.draw_arrays_instanced(u(1), int(2), int(3), int(4)),
                "drawElementsInstanced" => self.gl.draw_elements_instanced(u(1), int(2), u(3), int(4), int(5)),
                "readPixels" => {
                    let (x, y, w, h) = (int(1), int(2), int(3), int(4));
                    let mut buf = vec![0u8; (w.max(0) * h.max(0) * 4) as usize];
                    self.gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
                    self.gl.read_pixels(x, y, w, h, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut buf)));
                    reply.bytes = Some(buf);
                }
                "getError" => out = self.gl.get_error().to_string(),
                "present" => {
                    let (w, h) = (int(1), int(2));
                    self.resize(w, h);
                            let mut rgba = vec![0u8; (self.width * self.height * 4) as usize];
                    self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(self.fbo));
                    self.gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
                    self.gl.read_pixels(0, 0, self.width, self.height, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut rgba)));
                    flip_rows(&mut rgba, self.width as usize, self.height as usize, 4);
                    reply.frame = Some(Frame { flid: self.flid, width: self.width as u32, height: self.height as u32, rgba });
                }
                _ => return Err("unsupported".into()),
            }
        }
        Ok(out)
    }
}

/// `__zippHostCall("image_info", url)`: an image's size, for `naturalWidth`.
pub(crate) fn image_info(url: &str) -> Result<String, String> {
    let (status, raw) = crate::fetch_bytes_for_page(url)?;
    if !(200..300).contains(&status) {
        return Ok("{\"width\":0,\"height\":0}".into());
    }
    let size = image::ImageReader::new(std::io::Cursor::new(raw))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok())
        .unwrap_or((0, 0));
    Ok(format!("{{\"width\":{},\"height\":{}}}", size.0, size.1))
}
