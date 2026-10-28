//! WebGL 1 and 2 for `<canvas>`, on desktop OpenGL through WGL (Windows).
//!
//! The page side is `webgl_shim.js`: a `WebGLRenderingContext` (or
//! `WebGL2RenderingContext`) that records its calls into a batch of
//! `[op, ...args]` and sends it here with the `gl` host call, bytes beside it
//! in `__zippHostBytes`. A GL thread replays batches: one OpenGL context per
//! canvas, on a hidden window, each drawing into an offscreen framebuffer that
//! stands in for the canvas's default one (`bindFramebuffer(null)` binds it;
//! multisampled when the context is antialiased, as WebGL's are by default).
//! A `present` at the end of a frame reads that framebuffer back; the frame
//! goes to `canvas::take_frames`, and the page's canvas element gets it as its
//! image (`apply_canvas_frames` in main.rs).
//!
//! The GL thread keeps OpenGL off the window's thread, which never has a GL
//! context of its own. Each batch is synchronous: the page waits for the
//! answer to its last command (a query, or nothing).
//!
//! Shaders are translated one by one, since a WebGL 2 context takes both
//! kinds: GLSL ES 3.00 (`#version 300 es`) compiles as GLSL 3.30, and GLSL ES
//! 1.00 as GLSL 1.20, which has no precision qualifiers, so there
//! `lowp`/`mediump`/`highp` become nothing. `precision` statements go in both.
//! The context is a compatibility one, which runs both.
//!
//! What OpenGL ES always does and desktop OpenGL leaves to a switch is
//! switched on: point sprites and shader-set point sizes, seamless cube maps,
//! sRGB encoding when drawing into sRGB textures, and (WebGL 2) primitive
//! restart at the largest index.

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
    PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR, SetPixelFormat, wglCreateContext, wglDeleteContext, wglGetProcAddress,
    wglMakeCurrent,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryA};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{PCSTR, s, w};
use zipp_vm::embed::HostCtx;

use crate::canvas::Frame;

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
        crate::canvas::push_frame(frame);
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
    /// The canvas: multisampled (`samples` > 0) when antialiased, and then
    /// resolved into `resolve` to be read.
    fbo: glow::Framebuffer,
    color: glow::Renderbuffer,
    depth: glow::Renderbuffer,
    samples: i32,
    resolve: Option<(glow::Framebuffer, glow::Renderbuffer)>,
    /// `alpha: false`: the canvas is opaque whatever is drawn.
    opaque: bool,
    buffers: HashMap<u32, glow::Buffer>,
    textures: HashMap<u32, glow::Texture>,
    shaders: HashMap<u32, glow::Shader>,
    programs: HashMap<u32, glow::Program>,
    framebuffers: HashMap<u32, glow::Framebuffer>,
    renderbuffers: HashMap<u32, glow::Renderbuffer>,
    vaos: HashMap<u32, glow::VertexArray>,
    samplers: HashMap<u32, glow::Sampler>,
    queries: HashMap<u32, glow::Query>,
    feedbacks: HashMap<u32, glow::TransformFeedback>,
    uniforms: HashMap<(u32, String), Option<glow::UniformLocation>>,
    flip_y: bool,
    premultiply: bool,
    /// State the page set that resizing and reading back the canvas have to
    /// put back as they found it.
    state: PageState,
}

struct PageState {
    draw_fb: Option<glow::Framebuffer>,
    read_fb: Option<glow::Framebuffer>,
    renderbuffer: Option<glow::Renderbuffer>,
    pack_buffer: Option<glow::Buffer>,
    unpack_buffer: Option<glow::Buffer>,
    /// `pixelStorei` values other than WebGL's own flags.
    pixel_store: HashMap<u32, i32>,
    scissor_test: bool,
    clear_color: [f32; 4],
    clear_depth: f64,
    clear_stencil: i32,
    color_mask: [bool; 4],
    depth_mask: bool,
    stencil_mask: [u32; 2],
}

impl PageState {
    fn new(canvas: glow::Framebuffer) -> Self {
        PageState {
            draw_fb: Some(canvas),
            read_fb: Some(canvas),
            renderbuffer: None,
            pack_buffer: None,
            unpack_buffer: None,
            pixel_store: HashMap::new(),
            scissor_test: false,
            clear_color: [0.0; 4],
            clear_depth: 1.0,
            clear_stencil: 0,
            color_mask: [true; 4],
            depth_mask: true,
            stencil_mask: [u32::MAX; 2],
        }
    }

    fn pixel_store(&self, name: u32) -> i32 {
        let default = if matches!(name, glow::UNPACK_ALIGNMENT | glow::PACK_ALIGNMENT) { 4 } else { 0 };
        self.pixel_store.get(&name).copied().unwrap_or(default)
    }
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

    /// Delete a context and everything in it.
    fn delete(&mut self, context: u32) {
        if let Some(ctx) = self.contexts.remove(&context) {
            unsafe {
                if self.current == Some(context) {
                    let _ = wglMakeCurrent(self.dc, HGLRC::default());
                    self.current = None;
                }
                let _ = wglDeleteContext(ctx.hglrc);
            }
        }
    }

    fn create(&mut self, context: u32, cmd: &Json) -> Result<(), String> {
        let n = |i: usize| cmd.get(i).and_then(Json::as_f64).unwrap_or(0.0);
        let flag = |i: usize| cmd.get(i).and_then(Json::as_bool).unwrap_or(false);
        let (flid, width, height, version, antialias, alpha) =
            (n(1) as u32, n(2) as i32, n(3) as i32, n(4) as u32, flag(5), flag(6));
        // The page's numbers start again on every page: a context made under
        // a number still in use replaces it.
        self.delete(context);
        unsafe {
            let hglrc = wglCreateContext(self.dc).map_err(|e| e.to_string())?;
            wglMakeCurrent(self.dc, hglrc).map_err(|e| e.to_string())?;
            self.current = Some(context);
            let opengl32 = self.opengl32;
            let gl = glow::Context::from_loader_function(|name| Self::proc_address(opengl32, name));
            let fbo = gl.create_framebuffer()?;
            let color = gl.create_renderbuffer()?;
            let depth = gl.create_renderbuffer()?;
            let resolve = if antialias { Some((gl.create_framebuffer()?, gl.create_renderbuffer()?)) } else { None };
            for cap in [0x8861 /* POINT_SPRITE */, glow::PROGRAM_POINT_SIZE, glow::TEXTURE_CUBE_MAP_SEAMLESS, glow::FRAMEBUFFER_SRGB] {
                gl.enable(cap);
            }
            if version >= 2 {
                gl.enable(glow::PRIMITIVE_RESTART_FIXED_INDEX);
            }
            let mut ctx = Context {
                hglrc,
                gl,
                flid,
                width: 0,
                height: 0,
                fbo,
                color,
                depth,
                samples: if antialias { 4 } else { 0 },
                resolve,
                opaque: !alpha,
                buffers: HashMap::new(),
                textures: HashMap::new(),
                shaders: HashMap::new(),
                programs: HashMap::new(),
                framebuffers: HashMap::new(),
                renderbuffers: HashMap::new(),
                vaos: HashMap::new(),
                samplers: HashMap::new(),
                queries: HashMap::new(),
                feedbacks: HashMap::new(),
                uniforms: HashMap::new(),
                flip_y: false,
                premultiply: false,
                state: PageState::new(fbo),
            };
            ctx.resize(width, height);
            ctx.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            ctx.gl.viewport(0, 0, ctx.width, ctx.height);
            // Nothing the setup did is the page's error.
            while ctx.gl.get_error() != glow::NO_ERROR {}
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
            if op == "reset" {
                // A new page: the last one's contexts go.
                let all: Vec<u32> = self.contexts.keys().copied().collect();
                for c in all {
                    self.delete(c);
                }
                continue;
            }
            if op == "create" {
                if let Err(e) = self.create(context, cmd) {
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

/// A WebGL shader as desktop GLSL (see the module docs).
fn translate_shader(source: &str) -> String {
    let es3 = source.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("#version") && l.contains("300")
    });
    let mut body = String::with_capacity(source.len() + 256);
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#version") {
            continue;
        }
        // Built into desktop GLSL, or provided by the prelude below.
        if trimmed.starts_with("#extension")
            && ["GL_OES_standard_derivatives", "GL_EXT_frag_depth", "GL_EXT_shader_texture_lod", "GL_EXT_draw_buffers"]
                .iter()
                .any(|e| trimmed.contains(e))
        {
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
    if es3 {
        return format!("#version 330\n{body}");
    }
    let mut prelude = String::from("#version 120\n");
    if source.contains("GL_EXT_shader_texture_lod") {
        prelude.push_str(
            "#extension GL_ARB_shader_texture_lod : enable\n\
             #define texture2DLodEXT texture2DLod\n#define texture2DProjLodEXT texture2DProjLod\n\
             #define textureCubeLodEXT textureCubeLod\n#define texture2DGradEXT texture2DGradARB\n\
             #define texture2DProjGradEXT texture2DProjGradARB\n#define textureCubeGradEXT textureCubeGradARB\n",
        );
    }
    prelude.push_str("#define lowp\n#define mediump\n#define highp\n#define gl_FragDepthEXT gl_FragDepth\n");
    format!("{prelude}{body}")
}

/// Turn rows `stride` bytes apart upside down (GL reads the bottom row first).
fn flip_rows(data: &mut [u8], stride: usize, height: usize) {
    if stride == 0 {
        return;
    }
    for y in 0..height / 2 {
        let (top, bottom) = data.split_at_mut((height - 1 - y) * stride);
        top[y * stride..y * stride + stride].swap_with_slice(&mut bottom[..stride]);
    }
}

fn bytes_per_pixel(format: u32, ty: u32) -> usize {
    let channels = match format {
        glow::RGBA | glow::RGBA_INTEGER => 4,
        glow::RGB | glow::RGB_INTEGER => 3,
        glow::RG | glow::RG_INTEGER | glow::LUMINANCE_ALPHA => 2,
        _ => 1,
    };
    match ty {
        glow::FLOAT | glow::INT | glow::UNSIGNED_INT => channels * 4,
        glow::HALF_FLOAT | glow::SHORT | glow::UNSIGNED_SHORT => channels * 2,
        glow::UNSIGNED_SHORT_5_6_5 | glow::UNSIGNED_SHORT_4_4_4_4 | glow::UNSIGNED_SHORT_5_5_5_1 => 2,
        glow::UNSIGNED_INT_2_10_10_10_REV
        | glow::UNSIGNED_INT_10F_11F_11F_REV
        | glow::UNSIGNED_INT_5_9_9_9_REV
        | glow::UNSIGNED_INT_24_8 => 4,
        glow::FLOAT_32_UNSIGNED_INT_24_8_REV => 8,
        _ => channels,
    }
}

/// The page's name for the canvas's color buffer, as the offscreen one's.
fn canvas_buffer(buffer: u32) -> u32 {
    if buffer == glow::BACK { glow::COLOR_ATTACHMENT0 } else { buffer }
}

impl Context {
    /// Bytes between rows of a `width`-pixel image under the page's
    /// alignment and row length (`pack`: for reading, else for uploading).
    fn row_stride(&self, width: i32, format: u32, ty: u32, pack: bool) -> usize {
        let (align, length) = if pack {
            (glow::PACK_ALIGNMENT, glow::PACK_ROW_LENGTH)
        } else {
            (glow::UNPACK_ALIGNMENT, glow::UNPACK_ROW_LENGTH)
        };
        let pixels = match self.state.pixel_store(length) {
            0 => width,
            n => n,
        };
        let align = self.state.pixel_store(align).max(1) as usize;
        (pixels.max(0) as usize * bytes_per_pixel(format, ty)).div_ceil(align) * align
    }

    /// Page bytes for a `width`x`height` upload, flipped if UNPACK_FLIP_Y
    /// is set (padded out to whole rows first).
    fn unpack_bytes(&self, data: &[u8], width: i32, height: i32, format: u32, ty: u32) -> Vec<u8> {
        let mut data = data.to_vec();
        if self.flip_y && height > 1 {
            let stride = self.row_stride(width, format, ty, false);
            let rows = height as usize;
            if data.len() < stride * rows {
                data.resize(stride * rows, 0);
            }
            flip_rows(&mut data, stride, rows);
        }
        data
    }

    unsafe fn resize(&mut self, width: i32, height: i32) {
        let (width, height) = (width.max(1), height.max(1));
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        unsafe {
            let gl = &self.gl;
            let storage = |rb, format| {
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
                if self.samples > 0 {
                    gl.renderbuffer_storage_multisample(glow::RENDERBUFFER, self.samples, format, width, height);
                } else {
                    gl.renderbuffer_storage(glow::RENDERBUFFER, format, width, height);
                }
            };
            storage(self.color, glow::RGBA8);
            storage(self.depth, glow::DEPTH24_STENCIL8);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::RENDERBUFFER, Some(self.color));
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::DEPTH_STENCIL_ATTACHMENT, glow::RENDERBUFFER, Some(self.depth));
            if let Some((fb, rb)) = self.resolve {
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
                gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, width, height);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
                gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::RENDERBUFFER, Some(rb));
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            }
            // A new canvas is transparent black, whatever the page's clear
            // state, masks and scissor; those are put back after.
            gl.disable(glow::SCISSOR_TEST);
            gl.color_mask(true, true, true, true);
            gl.depth_mask(true);
            gl.stencil_mask(u32::MAX);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear_depth_f64(1.0);
            gl.clear_stencil(0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT | glow::STENCIL_BUFFER_BIT);
            self.restore_state();
        }
    }

    /// Put back what the page had bound and set.
    unsafe fn restore_state(&self) {
        let (gl, s) = (&self.gl, &self.state);
        unsafe {
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, s.draw_fb);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, s.read_fb);
            gl.bind_renderbuffer(glow::RENDERBUFFER, s.renderbuffer);
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, s.pack_buffer);
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, s.unpack_buffer);
            for name in [glow::PACK_ALIGNMENT, glow::UNPACK_ALIGNMENT, glow::PACK_ROW_LENGTH, glow::UNPACK_ROW_LENGTH] {
                gl.pixel_store_i32(name, s.pixel_store(name));
            }
            if s.scissor_test {
                gl.enable(glow::SCISSOR_TEST);
            }
            let [r, g, b, a] = s.color_mask;
            gl.color_mask(r, g, b, a);
            gl.depth_mask(s.depth_mask);
            gl.stencil_mask_separate(glow::FRONT, s.stencil_mask[0]);
            gl.stencil_mask_separate(glow::BACK, s.stencil_mask[1]);
            let [r, g, b, a] = s.clear_color;
            gl.clear_color(r, g, b, a);
            gl.clear_depth_f64(s.clear_depth);
            gl.clear_stencil(s.clear_stencil);
        }
    }

    /// When the page reads from the canvas and it is multisampled, resolve it
    /// and read from that instead. True when it did (`end_resolved_read`
    /// puts the read binding back).
    unsafe fn begin_resolved_read(&self) -> bool {
        let Some((resolve, _)) = self.resolve else {
            return false;
        };
        if self.state.read_fb != Some(self.fbo) {
            return false;
        }
        unsafe {
            self.resolve_canvas(resolve);
            self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(resolve));
        }
        true
    }

    unsafe fn end_resolved_read(&self, resolved: bool) {
        if resolved {
            unsafe { self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, self.state.read_fb) };
        }
    }

    /// Blit the multisampled canvas into `resolve`, leaving the page's draw
    /// binding and scissor as they were.
    unsafe fn resolve_canvas(&self, resolve: glow::Framebuffer) {
        let gl = &self.gl;
        unsafe {
            gl.disable(glow::SCISSOR_TEST);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(self.fbo));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(resolve));
            let (w, h) = (self.width, self.height);
            gl.blit_framebuffer(0, 0, w, h, 0, 0, w, h, glow::COLOR_BUFFER_BIT, glow::NEAREST);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, self.state.draw_fb);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, self.state.read_fb);
            if self.state.scissor_test {
                gl.enable(glow::SCISSOR_TEST);
            }
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

    /// A program's attributes as WebGL lists them: without built-ins
    /// (`gl_VertexID`), which some drivers count.
    unsafe fn active_attributes(&self, p: glow::Program) -> Vec<glow::ActiveAttribute> {
        unsafe {
            (0..self.gl.get_active_attributes(p))
                .filter_map(|i| self.gl.get_active_attribute(p, i))
                .filter(|a| !a.name.starts_with("gl_"))
                .collect()
        }
    }

    /// An image's pixels from its URL, as RGBA8 with the unpack flags applied.
    fn decode_image(&self, url: &str) -> Result<(i32, i32, Vec<u8>), String> {
        let (_, raw) = crate::fetch_bytes_for_page(url).map_err(|e| e.to_string())?;
        let img = image::load_from_memory(&raw).map_err(|e| format!("texture {url}: {e}"))?;
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width() as i32, rgba.height() as i32);
        let mut rgba = rgba.into_raw();
        if self.flip_y {
            flip_rows(&mut rgba, w as usize * 4, h as usize);
        }
        if self.premultiply {
            for px in rgba.chunks_exact_mut(4) {
                let a = px[3] as u32;
                for c in &mut px[..3] {
                    *c = ((*c as u32 * a + 127) / 255) as u8;
                }
            }
        }
        Ok((w, h, rgba))
    }

    /// Upload decoded RGBA8 rows, tightly packed, from client memory whatever
    /// the page set for its own uploads.
    unsafe fn with_tight_unpack(&self, upload: impl FnOnce(&glow::Context)) {
        let gl = &self.gl;
        unsafe {
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
            for name in [glow::UNPACK_ROW_LENGTH, glow::UNPACK_SKIP_ROWS, glow::UNPACK_SKIP_PIXELS] {
                gl.pixel_store_i32(name, 0);
            }
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            upload(gl);
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, self.state.unpack_buffer);
            for name in [glow::UNPACK_ROW_LENGTH, glow::UNPACK_SKIP_ROWS, glow::UNPACK_SKIP_PIXELS, glow::UNPACK_ALIGNMENT] {
                gl.pixel_store_i32(name, self.state.pixel_store(name));
            }
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
        let list = |i: usize| cmd.get(i).and_then(Json::as_array).cloned().unwrap_or_default();
        let nums = |i: usize| -> Vec<f64> { list(i).iter().filter_map(Json::as_f64).collect() };
        let texts = |i: usize| -> Vec<String> { list(i).iter().filter_map(|v| v.as_str().map(String::from)).collect() };
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
                "bindBuffer" => {
                    let b = self.buffers.get(&u(2)).copied();
                    match u(1) {
                        glow::PIXEL_PACK_BUFFER => self.state.pack_buffer = b,
                        glow::PIXEL_UNPACK_BUFFER => self.state.unpack_buffer = b,
                        _ => {}
                    }
                    self.gl.bind_buffer(u(1), b);
                }
                "bindBufferBase" => self.gl.bind_buffer_base(u(1), u(2), self.buffers.get(&u(3)).copied()),
                "bindBufferRange" => {
                    self.gl.bind_buffer_range(u(1), u(2), self.buffers.get(&u(3)).copied(), int(4), int(5));
                }
                "bufferData" => self.gl.buffer_data_u8_slice(u(1), bytes(u(2) as usize, u(3) as usize)?, u(4)),
                "bufferSize" => self.gl.buffer_data_size(u(1), int(2), u(3)),
                "bufferSubData" => self.gl.buffer_sub_data_u8_slice(u(1), int(2), bytes(u(3) as usize, u(4) as usize)?),
                "copyBufferSubData" => self.gl.copy_buffer_sub_data(u(1), u(2), int(3), int(4), int(5)),
                "getBufferSubData" => {
                    let mut buf = vec![0u8; u(3) as usize];
                    self.gl.get_buffer_sub_data(u(1), int(2), &mut buf);
                    reply.bytes = Some(buf);
                }
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
                // A driver's warnings about a shader that compiled are not
                // WebGL's to report (Chrome's translator has none), and pages
                // print a non-empty log: only failures keep theirs.
                "getShaderInfoLog" => {
                    let log = match self.shaders.get(&u(1)) {
                        Some(&s) if !self.gl.get_shader_compile_status(s) => self.gl.get_shader_info_log(s),
                        _ => String::new(),
                    };
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
                        (Some(p), glow::ACTIVE_ATTRIBUTES) => self.active_attributes(p).len().to_string(),
                        (Some(p), glow::ACTIVE_UNIFORMS) => self.gl.get_active_uniforms(p).to_string(),
                        (Some(p), pname @ (glow::ACTIVE_UNIFORM_BLOCKS | glow::TRANSFORM_FEEDBACK_VARYINGS
                            | glow::TRANSFORM_FEEDBACK_BUFFER_MODE)) => {
                            self.gl.get_program_parameter_i32(p, pname).to_string()
                        }
                        (Some(_), glow::VALIDATE_STATUS) => "true".into(),
                        (_, glow::DELETE_STATUS) => "false".into(),
                        (Some(_), glow::ATTACHED_SHADERS) => "2".into(),
                        _ => "null".into(),
                    };
                }
                "getProgramInfoLog" => {
                    let log = match self.programs.get(&u(1)) {
                        Some(&p) if !self.gl.get_program_link_status(p) => self.gl.get_program_info_log(p),
                        _ => String::new(),
                    };
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
                            self.active_attributes(p).into_iter().nth(u(2) as usize).map(|a| (a.name, a.size, a.atype))
                        } else {
                            self.gl.get_active_uniform(p, u(2)).map(|a| (a.name, a.size, a.utype))
                        }
                    });
                    out = match info {
                        Some((name, size, ty)) => serde_json::json!([name, size, ty]).to_string(),
                        None => "null".into(),
                    };
                }
                "getFragDataLocation" => {
                    let at = self.programs.get(&u(1)).map(|&p| self.gl.get_frag_data_location(p, &text(2)));
                    out = at.unwrap_or(-1).to_string();
                }
                "getUniformBlockIndex" => {
                    let index = self.programs.get(&u(1)).and_then(|&p| self.gl.get_uniform_block_index(p, &text(2)));
                    out = index.unwrap_or(u32::MAX).to_string();
                }
                "uniformBlockBinding" => {
                    if let Some(&p) = self.programs.get(&u(1)) {
                        self.gl.uniform_block_binding(p, u(2), u(3));
                    }
                }
                "getActiveUniformBlockParameter" => {
                    let Some(&p) = self.programs.get(&u(1)) else { return Ok("null".into()) };
                    let (index, pname) = (u(2), u(3));
                    out = match pname {
                        glow::UNIFORM_BLOCK_ACTIVE_UNIFORM_INDICES => {
                            let n = self.gl.get_active_uniform_block_parameter_i32(p, index, glow::UNIFORM_BLOCK_ACTIVE_UNIFORMS);
                            let mut indices = vec![0i32; n.max(0) as usize];
                            self.gl.get_active_uniform_block_parameter_i32_slice(p, index, pname, &mut indices);
                            serde_json::json!(indices).to_string()
                        }
                        glow::UNIFORM_BLOCK_REFERENCED_BY_VERTEX_SHADER | glow::UNIFORM_BLOCK_REFERENCED_BY_FRAGMENT_SHADER => {
                            (self.gl.get_active_uniform_block_parameter_i32(p, index, pname) != 0).to_string()
                        }
                        _ => self.gl.get_active_uniform_block_parameter_i32(p, index, pname).to_string(),
                    };
                }
                "getActiveUniformBlockName" => {
                    let name = self.programs.get(&u(1)).map(|&p| self.gl.get_active_uniform_block_name(p, u(2)));
                    out = name.map(|n| Json::String(n).to_string()).unwrap_or_else(|| "null".into());
                }
                "getUniformIndices" => {
                    let names = texts(2);
                    let names: Vec<&str> = names.iter().map(String::as_str).collect();
                    let indices = self.programs.get(&u(1)).map(|&p| self.gl.get_uniform_indices(p, &names));
                    out = match indices {
                        Some(i) => serde_json::json!(i.iter().map(|i| i.unwrap_or(u32::MAX)).collect::<Vec<_>>()).to_string(),
                        None => "null".into(),
                    };
                }
                "getActiveUniforms" => {
                    let indices: Vec<u32> = nums(2).iter().map(|&i| i as u32).collect();
                    let pname = u(3);
                    let values = self.programs.get(&u(1)).map(|&p| self.gl.get_active_uniforms_parameter(p, &indices, pname));
                    out = match values {
                        Some(v) if pname == glow::UNIFORM_IS_ROW_MAJOR => {
                            serde_json::json!(v.iter().map(|&x| x != 0).collect::<Vec<_>>()).to_string()
                        }
                        Some(v) => serde_json::json!(v).to_string(),
                        None => "null".into(),
                    };
                }
                "enableVertexAttribArray" => self.gl.enable_vertex_attrib_array(u(1)),
                "disableVertexAttribArray" => self.gl.disable_vertex_attrib_array(u(1)),
                "vertexAttribPointer" => self.gl.vertex_attrib_pointer_f32(u(1), int(2), u(3), flag(4), int(5), int(6)),
                "vertexAttribIPointer" => self.gl.vertex_attrib_pointer_i32(u(1), int(2), u(3), int(4), int(5)),
                "vertexAttrib" => self.gl.vertex_attrib_4_f32(u(1), f(2), f(3), f(4), f(5)),
                "vertexAttribI" => {
                    if flag(6) {
                        self.gl.vertex_attrib_4_u32(u(1), u(2), u(3), u(4), u(5));
                    } else {
                        self.gl.vertex_attrib_4_i32(u(1), int(2), int(3), int(4), int(5));
                    }
                }
                "vertexAttribDivisor" => self.gl.vertex_attrib_divisor(u(1), u(2)),
                "uniform" => {
                    let (program, name, kind, transpose) = (u(1), text(2), text(3), flag(5));
                    let values = nums(4);
                    if let Some(loc) = self.uniform_location(program, &name) {
                        let fs: Vec<f32> = values.iter().map(|&v| v as f32).collect();
                        let is: Vec<i32> = values.iter().map(|&v| v as i32).collect();
                        let us: Vec<u32> = values.iter().map(|&v| v as u32).collect();
                        let (gl, loc) = (&self.gl, Some(&loc));
                        match kind.as_str() {
                            "1f" => gl.uniform_1_f32_slice(loc, &fs),
                            "2f" => gl.uniform_2_f32_slice(loc, &fs),
                            "3f" => gl.uniform_3_f32_slice(loc, &fs),
                            "4f" => gl.uniform_4_f32_slice(loc, &fs),
                            "1i" => gl.uniform_1_i32_slice(loc, &is),
                            "2i" => gl.uniform_2_i32_slice(loc, &is),
                            "3i" => gl.uniform_3_i32_slice(loc, &is),
                            "4i" => gl.uniform_4_i32_slice(loc, &is),
                            "1u" => gl.uniform_1_u32_slice(loc, &us),
                            "2u" => gl.uniform_2_u32_slice(loc, &us),
                            "3u" => gl.uniform_3_u32_slice(loc, &us),
                            "4u" => gl.uniform_4_u32_slice(loc, &us),
                            "m2" => gl.uniform_matrix_2_f32_slice(loc, transpose, &fs),
                            "m3" => gl.uniform_matrix_3_f32_slice(loc, transpose, &fs),
                            "m4" => gl.uniform_matrix_4_f32_slice(loc, transpose, &fs),
                            "m23" => gl.uniform_matrix_2x3_f32_slice(loc, transpose, &fs),
                            "m24" => gl.uniform_matrix_2x4_f32_slice(loc, transpose, &fs),
                            "m32" => gl.uniform_matrix_3x2_f32_slice(loc, transpose, &fs),
                            "m34" => gl.uniform_matrix_3x4_f32_slice(loc, transpose, &fs),
                            "m42" => gl.uniform_matrix_4x2_f32_slice(loc, transpose, &fs),
                            "m43" => gl.uniform_matrix_4x3_f32_slice(loc, transpose, &fs),
                            _ => {}
                        }
                    }
                }
                "createTexture" => { let t = self.gl.create_texture()?; self.textures.insert(u(1), t); }
                "deleteTexture" => { if let Some(t) = self.textures.remove(&u(1)) { self.gl.delete_texture(t); } }
                "bindTexture" => self.gl.bind_texture(u(1), self.textures.get(&u(2)).copied()),
                "activeTexture" => self.gl.active_texture(u(1)),
                "texParameteri" => self.gl.tex_parameter_i32(u(1), u(2), int(3)),
                "texParameterf" => self.gl.tex_parameter_f32(u(1), u(2), f(3)),
                "pixelStorei" => match u(1) {
                    0x9240 => self.flip_y = num(2) != 0.0,
                    0x9241 => self.premultiply = num(2) != 0.0,
                    0x9243 => {}
                    name => {
                        self.state.pixel_store.insert(name, int(2));
                        self.gl.pixel_store_i32(name, int(2));
                    }
                },
                "generateMipmap" => self.gl.generate_mipmap(u(1)),
                "texStorage2D" => self.gl.tex_storage_2d(u(1), int(2), u(3), int(4), int(5)),
                "texStorage3D" => self.gl.tex_storage_3d(u(1), int(2), u(3), int(4), int(5), int(6)),
                "texImage2D" => {
                    let (target, level, internal, w, h, format, ty) = (u(1), int(2), int(3), int(4), int(5), u(6), u(7));
                    let at = num(8);
                    if at < 0.0 {
                        self.gl.tex_image_2d(target, level, internal, w, h, 0, format, ty, glow::PixelUnpackData::Slice(None));
                    } else {
                        let data = self.unpack_bytes(bytes(at as usize, u(9) as usize)?, w, h, format, ty);
                        self.gl.tex_image_2d(target, level, internal, w, h, 0, format, ty, glow::PixelUnpackData::Slice(Some(&data)));
                    }
                }
                "texImage2DPbo" => {
                    let pixels = glow::PixelUnpackData::BufferOffset(u(8));
                    self.gl.tex_image_2d(u(1), int(2), int(3), int(4), int(5), 0, u(6), u(7), pixels);
                }
                "texImage2DUrl" => {
                    let (target, level, internal, url) = (u(1), int(2), int(3), text(6));
                    let (w, h, rgba) = self.decode_image(&url)?;
                    self.with_tight_unpack(|gl| {
                        let pixels = glow::PixelUnpackData::Slice(Some(&rgba));
                        gl.tex_image_2d(target, level, internal, w, h, 0, glow::RGBA, glow::UNSIGNED_BYTE, pixels);
                    });
                }
                "texSubImage2D" => {
                    let (target, level, x, y, w, h, format, ty) = (u(1), int(2), int(3), int(4), int(5), int(6), u(7), u(8));
                    let data = self.unpack_bytes(bytes(u(9) as usize, u(10) as usize)?, w, h, format, ty);
                    self.gl.tex_sub_image_2d(target, level, x, y, w, h, format, ty, glow::PixelUnpackData::Slice(Some(&data)));
                }
                "texSubImage2DPbo" => {
                    let pixels = glow::PixelUnpackData::BufferOffset(u(9));
                    self.gl.tex_sub_image_2d(u(1), int(2), int(3), int(4), int(5), int(6), u(7), u(8), pixels);
                }
                "texSubImage2DUrl" => {
                    let (target, level, x, y, url) = (u(1), int(2), int(3), int(4), text(7));
                    let (w, h, rgba) = self.decode_image(&url)?;
                    self.with_tight_unpack(|gl| {
                        let pixels = glow::PixelUnpackData::Slice(Some(&rgba));
                        gl.tex_sub_image_2d(target, level, x, y, w, h, glow::RGBA, glow::UNSIGNED_BYTE, pixels);
                    });
                }
                "texImage3D" => {
                    let (target, level, internal, w, h, d, format, ty) = (u(1), int(2), int(3), int(4), int(5), int(6), u(7), u(8));
                    let at = num(9);
                    let data = if at < 0.0 { None } else { Some(bytes(at as usize, u(10) as usize)?) };
                    self.gl.tex_image_3d(target, level, internal, w, h, d, 0, format, ty, glow::PixelUnpackData::Slice(data));
                }
                "texImage3DPbo" => {
                    let pixels = glow::PixelUnpackData::BufferOffset(u(9));
                    self.gl.tex_image_3d(u(1), int(2), int(3), int(4), int(5), int(6), 0, u(7), u(8), pixels);
                }
                "texSubImage3D" => {
                    let data = bytes(u(11) as usize, u(12) as usize)?;
                    let pixels = glow::PixelUnpackData::Slice(Some(data));
                    self.gl.tex_sub_image_3d(u(1), int(2), int(3), int(4), int(5), int(6), int(7), int(8), u(9), u(10), pixels);
                }
                "texSubImage3DPbo" => {
                    let pixels = glow::PixelUnpackData::BufferOffset(u(11));
                    self.gl.tex_sub_image_3d(u(1), int(2), int(3), int(4), int(5), int(6), int(7), int(8), u(9), u(10), pixels);
                }
                "copyTexImage2D" => {
                    let resolved = self.begin_resolved_read();
                    self.gl.copy_tex_image_2d(u(1), int(2), u(3), int(4), int(5), int(6), int(7), 0);
                    self.end_resolved_read(resolved);
                }
                "copyTexSubImage2D" => {
                    let resolved = self.begin_resolved_read();
                    self.gl.copy_tex_sub_image_2d(u(1), int(2), int(3), int(4), int(5), int(6), int(7), int(8));
                    self.end_resolved_read(resolved);
                }
                "copyTexSubImage3D" => {
                    let resolved = self.begin_resolved_read();
                    self.gl.copy_tex_sub_image_3d(u(1), int(2), int(3), int(4), int(5), int(6), int(7), int(8), int(9));
                    self.end_resolved_read(resolved);
                }
                "createSampler" => { let s = self.gl.create_sampler()?; self.samplers.insert(u(1), s); }
                "deleteSampler" => { if let Some(s) = self.samplers.remove(&u(1)) { self.gl.delete_sampler(s); } }
                "bindSampler" => self.gl.bind_sampler(u(1), self.samplers.get(&u(2)).copied()),
                "samplerParameteri" => {
                    if let Some(&s) = self.samplers.get(&u(1)) { self.gl.sampler_parameter_i32(s, u(2), int(3)); }
                }
                "samplerParameterf" => {
                    if let Some(&s) = self.samplers.get(&u(1)) { self.gl.sampler_parameter_f32(s, u(2), f(3)); }
                }
                "createFramebuffer" => { let fb = self.gl.create_framebuffer()?; self.framebuffers.insert(u(1), fb); }
                "deleteFramebuffer" => {
                    if let Some(fb) = self.framebuffers.remove(&u(1)) {
                        self.gl.delete_framebuffer(fb);
                        // Deleting a bound framebuffer binds the canvas's.
                        let canvas = Some(self.fbo);
                        for bound in [&mut self.state.draw_fb, &mut self.state.read_fb] {
                            if *bound == Some(fb) {
                                *bound = canvas;
                            }
                        }
                        self.gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, self.state.draw_fb);
                        self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, self.state.read_fb);
                    }
                }
                "bindFramebuffer" => {
                    // Null is the canvas: its offscreen framebuffer.
                    let fb = if u(2) == 0 { Some(self.fbo) } else { self.framebuffers.get(&u(2)).copied() };
                    let target = u(1);
                    if target != glow::READ_FRAMEBUFFER {
                        self.state.draw_fb = fb;
                    }
                    if target != glow::DRAW_FRAMEBUFFER {
                        self.state.read_fb = fb;
                    }
                    self.gl.bind_framebuffer(target, fb);
                }
                "framebufferTexture2D" => {
                    self.gl.framebuffer_texture_2d(u(1), u(2), u(3), self.textures.get(&u(4)).copied(), int(5));
                }
                "framebufferTextureLayer" => {
                    self.gl.framebuffer_texture_layer(u(1), u(2), self.textures.get(&u(3)).copied(), int(4), int(5));
                }
                "framebufferRenderbuffer" => {
                    self.gl.framebuffer_renderbuffer(u(1), u(2), u(3), self.renderbuffers.get(&u(4)).copied());
                }
                "checkFramebufferStatus" => out = self.gl.check_framebuffer_status(u(1)).to_string(),
                "drawBuffers" => {
                    let buffers: Vec<u32> = nums(1).iter().map(|&b| canvas_buffer(b as u32)).collect();
                    self.gl.draw_buffers(&buffers);
                }
                "readBuffer" => self.gl.read_buffer(canvas_buffer(u(1))),
                "blitFramebuffer" => {
                    let resolved = self.begin_resolved_read();
                    self.gl.blit_framebuffer(int(1), int(2), int(3), int(4), int(5), int(6), int(7), int(8), u(9), u(10));
                    self.end_resolved_read(resolved);
                }
                "createRenderbuffer" => { let rb = self.gl.create_renderbuffer()?; self.renderbuffers.insert(u(1), rb); }
                "deleteRenderbuffer" => {
                    if let Some(rb) = self.renderbuffers.remove(&u(1)) {
                        self.gl.delete_renderbuffer(rb);
                        if self.state.renderbuffer == Some(rb) {
                            self.state.renderbuffer = None;
                        }
                    }
                }
                "bindRenderbuffer" => {
                    let rb = self.renderbuffers.get(&u(2)).copied();
                    self.state.renderbuffer = rb;
                    self.gl.bind_renderbuffer(u(1), rb);
                }
                "renderbufferStorage" => {
                    // WebGL's DEPTH_STENCIL is a sized format on the desktop.
                    let format = if u(2) == 0x84f9 { glow::DEPTH24_STENCIL8 } else { u(2) };
                    self.gl.renderbuffer_storage(u(1), format, int(3), int(4));
                }
                "renderbufferStorageMultisample" => {
                    let format = if u(3) == 0x84f9 { glow::DEPTH24_STENCIL8 } else { u(3) };
                    self.gl.renderbuffer_storage_multisample(u(1), int(2), format, int(4), int(5));
                }
                "createVertexArray" => { let v = self.gl.create_vertex_array()?; self.vaos.insert(u(1), v); }
                "deleteVertexArray" => { if let Some(v) = self.vaos.remove(&u(1)) { self.gl.delete_vertex_array(v); } }
                "bindVertexArray" => self.gl.bind_vertex_array(self.vaos.get(&u(1)).copied()),
                "createQuery" => { let q = self.gl.create_query()?; self.queries.insert(u(1), q); }
                "deleteQuery" => { if let Some(q) = self.queries.remove(&u(1)) { self.gl.delete_query(q); } }
                "beginQuery" => {
                    if let Some(&q) = self.queries.get(&u(2)) { self.gl.begin_query(u(1), q); }
                }
                "endQuery" => self.gl.end_query(u(1)),
                "getQueryParameter" => {
                    let Some(&q) = self.queries.get(&u(1)) else { return Ok("null".into()) };
                    let value = self.gl.get_query_parameter_u32(q, u(2));
                    out = if u(2) == glow::QUERY_RESULT_AVAILABLE { (value != 0).to_string() } else { value.to_string() };
                }
                "createTransformFeedback" => {
                    let t = self.gl.create_transform_feedback()?;
                    self.feedbacks.insert(u(1), t);
                }
                "deleteTransformFeedback" => {
                    if let Some(t) = self.feedbacks.remove(&u(1)) { self.gl.delete_transform_feedback(t); }
                }
                "bindTransformFeedback" => self.gl.bind_transform_feedback(u(1), self.feedbacks.get(&u(2)).copied()),
                "beginTransformFeedback" => self.gl.begin_transform_feedback(u(1)),
                "endTransformFeedback" => self.gl.end_transform_feedback(),
                "pauseTransformFeedback" => self.gl.pause_transform_feedback(),
                "resumeTransformFeedback" => self.gl.resume_transform_feedback(),
                "transformFeedbackVaryings" => {
                    if let Some(&p) = self.programs.get(&u(1)) {
                        let names = texts(2);
                        let names: Vec<&str> = names.iter().map(String::as_str).collect();
                        self.gl.transform_feedback_varyings(p, &names, u(3));
                    }
                }
                "getTransformFeedbackVarying" => {
                    let info = self.programs.get(&u(1)).and_then(|&p| self.gl.get_transform_feedback_varying(p, u(2)));
                    out = match info {
                        Some(v) => serde_json::json!([v.name, v.size, v.tftype]).to_string(),
                        None => "null".into(),
                    };
                }
                "enable" | "disable" => {
                    let on = op == "enable";
                    if u(1) == glow::SCISSOR_TEST {
                        self.state.scissor_test = on;
                    }
                    if on { self.gl.enable(u(1)) } else { self.gl.disable(u(1)) }
                }
                "viewport" => self.gl.viewport(int(1), int(2), int(3), int(4)),
                "scissor" => self.gl.scissor(int(1), int(2), int(3), int(4)),
                "clearColor" => {
                    self.state.clear_color = [f(1), f(2), f(3), f(4)];
                    self.gl.clear_color(f(1), f(2), f(3), f(4));
                }
                "clearDepth" => {
                    self.state.clear_depth = num(1);
                    self.gl.clear_depth_f64(num(1));
                }
                "clearStencil" => {
                    self.state.clear_stencil = int(1);
                    self.gl.clear_stencil(int(1));
                }
                "clear" => self.gl.clear(u(1)),
                "clearBufferfv" => {
                    let v: Vec<f32> = nums(3).iter().map(|&x| x as f32).collect();
                    self.gl.clear_buffer_f32_slice(u(1), u(2), &v);
                }
                "clearBufferiv" => {
                    let v: Vec<i32> = nums(3).iter().map(|&x| x as i32).collect();
                    self.gl.clear_buffer_i32_slice(u(1), u(2), &v);
                }
                "clearBufferuiv" => {
                    let v: Vec<u32> = nums(3).iter().map(|&x| x as u32).collect();
                    self.gl.clear_buffer_u32_slice(u(1), u(2), &v);
                }
                "clearBufferfi" => self.gl.clear_buffer_depth_stencil(u(1), u(2), f(3), int(4)),
                "colorMask" => {
                    self.state.color_mask = [flag(1), flag(2), flag(3), flag(4)];
                    self.gl.color_mask(flag(1), flag(2), flag(3), flag(4));
                }
                "depthMask" => {
                    self.state.depth_mask = flag(1);
                    self.gl.depth_mask(flag(1));
                }
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
                "stencilMaskSeparate" => {
                    let (face, mask) = (u(1), u(2));
                    if face != glow::BACK {
                        self.state.stencil_mask[0] = mask;
                    }
                    if face != glow::FRONT {
                        self.state.stencil_mask[1] = mask;
                    }
                    self.gl.stencil_mask_separate(face, mask);
                }
                "drawArrays" => self.gl.draw_arrays(u(1), int(2), int(3)),
                "drawElements" => self.gl.draw_elements(u(1), int(2), u(3), int(4)),
                "drawArraysInstanced" => self.gl.draw_arrays_instanced(u(1), int(2), int(3), int(4)),
                "drawElementsInstanced" => self.gl.draw_elements_instanced(u(1), int(2), u(3), int(4), int(5)),
                "readPixels" => {
                    let (x, y, w, h, format, ty) = (int(1), int(2), int(3), int(4), u(5), u(6));
                    let resolved = self.begin_resolved_read();
                    if num(7) >= 0.0 {
                        let pixels = glow::PixelPackData::BufferOffset(u(7));
                        self.gl.read_pixels(x, y, w, h, format, ty, pixels);
                    } else {
                        let len = self.row_stride(w, format, ty, true) * h.max(0) as usize;
                        let mut buf = vec![0u8; len];
                        self.gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
                        self.gl.read_pixels(x, y, w, h, format, ty, glow::PixelPackData::Slice(Some(&mut buf)));
                        self.gl.bind_buffer(glow::PIXEL_PACK_BUFFER, self.state.pack_buffer);
                        out = len.to_string();
                        reply.bytes = Some(buf);
                    }
                    self.end_resolved_read(resolved);
                }
                "finish" => self.gl.finish(),
                "getError" => out = self.gl.get_error().to_string(),
                "present" => {
                    let (w, h) = (int(1), int(2));
                    self.resize(w, h);
                    let source = match self.resolve {
                        Some((resolve, _)) => {
                            self.resolve_canvas(resolve);
                            resolve
                        }
                        None => self.fbo,
                    };
                    let mut rgba = vec![0u8; (self.width * self.height * 4) as usize];
                    let gl = &self.gl;
                    gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(source));
                    gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
                    gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
                    gl.pixel_store_i32(glow::PACK_ROW_LENGTH, 0);
                    gl.read_pixels(0, 0, self.width, self.height, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut rgba)));
                    self.restore_state();
                    flip_rows(&mut rgba, self.width as usize * 4, self.height as usize);
                    if self.opaque {
                        for px in rgba.chunks_exact_mut(4) {
                            px[3] = 255;
                        }
                    }
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

#[cfg(test)]
mod tests {
    use super::translate_shader;

    #[test]
    fn es3_shaders_compile_as_glsl_330() {
        let out = translate_shader("#version 300 es\nprecision highp float;\nout highp vec4 c;\nvoid main(){ c = vec4(1); }");
        assert!(out.starts_with("#version 330\n"), "{out}");
        assert!(!out.contains("precision highp float;"), "{out}");
        assert!(out.contains("out highp vec4 c;"), "qualifiers on declarations stay: {out}");
        assert!(!out.contains("#define highp"), "{out}");
    }

    #[test]
    fn es1_shaders_compile_as_glsl_120() {
        let out = translate_shader("#extension GL_OES_standard_derivatives : enable\nprecision mediump float;\nvoid main(){}");
        assert!(out.starts_with("#version 120\n"), "{out}");
        assert!(out.contains("#define mediump\n"), "{out}");
        assert!(!out.contains("GL_OES_standard_derivatives"), "{out}");
        assert!(!out.contains("precision mediump float;"), "{out}");
    }
}
