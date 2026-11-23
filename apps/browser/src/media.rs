//! `<video>` playback on Windows, through Media Foundation's media engine.
//!
//! The media engine is the component Windows provides for HTML media: it
//! fetches the resource, demuxes and decodes it with whatever codecs the system
//! has (H.264, VP9, AV1, MPEG-4, ... in MP4, WebM and QuickTime), plays the
//! audio and keeps it in sync. It runs in "frame server" mode: each new frame is
//! transferred into a D3D11 texture, copied back to memory and handed to the
//! document as an image, so it is painted -- and clipped, scrolled and drawn
//! over -- like any other.
//!
//! Each player has its own worker thread, which owns all the COM objects; the
//! document talks to it through a channel and reads its latest frame and status
//! from shared state. Dropping the player tells the worker to shut down.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::{BSTR, Interface};
use windows_core::implement;

use zipp_dom::node::RasterImageData;
use zipp_dom::traits_src::media::{MediaOptions, MediaPlayer, MediaProvider, MediaSource, MediaStatus};
use zipp_dom::traits_src::shell::ShellProvider;

/// Frames are copied out at no more than this width, whatever the display size.
const MAX_FRAME_WIDTH: u32 = 1920;
/// The most the engine is asked to shrink a frame as it transfers it. GPU video
/// processors cap their downscale ratio, and past it the engine reports a decode
/// error rather than a scaled frame: Wikipedia's 2558px original in a 250px
/// box (10x) failed so. The painter scales the rest of the way.
const MAX_ENGINE_DOWNSCALE: u32 = 4;

/// The [`MediaProvider`] for windows the browser opens.
pub struct MediaFoundationProvider {
    /// `ZIPP_MUTE_MEDIA=1`: every player starts muted. For automated runs on a
    /// machine whose speakers someone is listening to.
    force_muted: bool,
}

impl MediaFoundationProvider {
    pub fn new() -> Self {
        Self { force_muted: std::env::var_os("ZIPP_MUTE_MEDIA").is_some() }
    }
}

impl MediaProvider for MediaFoundationProvider {
    fn create_player(
        &self,
        sources: Vec<MediaSource>,
        mut options: MediaOptions,
        shell: Arc<dyn ShellProvider>,
    ) -> Option<Arc<dyn MediaPlayer>> {
        options.muted |= self.force_muted;
        let shared = Arc::new(Shared {
            frame: Mutex::new(None),
            display_size: Mutex::new(None),
            status: Mutex::new(MediaStatus { muted: options.muted, ..Default::default() }),
            shell: Mutex::new(shell),
        });
        let (tx, rx) = mpsc::channel();
        let worker_shared = shared.clone();
        std::thread::Builder::new()
            .name("zipp-media".into())
            .spawn(move || worker(sources, options, worker_shared, rx))
            .ok()?;
        Some(Arc::new(Player { commands: Mutex::new(tx), shared }))
    }
}

enum Command {
    Play,
    Pause,
    Seek(f64),
    Mute(bool),
}

/// What the worker publishes for the document to read.
struct Shared {
    frame: Mutex<Option<RasterImageData>>,
    /// The size the video is painted at, in device pixels.
    display_size: Mutex<Option<(u32, u32)>>,
    status: Mutex<MediaStatus>,
    shell: Mutex<Arc<dyn ShellProvider>>,
}

impl Shared {
    fn shell(&self) -> Arc<dyn ShellProvider> {
        self.shell.lock().unwrap().clone()
    }
}

struct Player {
    commands: Mutex<Sender<Command>>,
    shared: Arc<Shared>,
}

impl Player {
    fn send(&self, command: Command) {
        let _ = self.commands.lock().unwrap().send(command);
    }
}

impl MediaPlayer for Player {
    fn play(&self) {
        self.send(Command::Play);
    }
    fn pause(&self) {
        self.send(Command::Pause);
    }
    fn seek(&self, seconds: f64) {
        self.send(Command::Seek(seconds));
    }
    fn set_muted(&self, muted: bool) {
        self.shared.status.lock().unwrap().muted = muted;
        self.send(Command::Mute(muted));
    }
    fn status(&self) -> MediaStatus {
        *self.shared.status.lock().unwrap()
    }
    fn frame(&self) -> Option<RasterImageData> {
        self.shared.frame.lock().unwrap().clone()
    }
    fn set_shell_provider(&self, shell: Arc<dyn ShellProvider>) {
        *self.shared.shell.lock().unwrap() = shell;
    }
    fn set_display_size(&self, width: u32, height: u32) {
        *self.shared.display_size.lock().unwrap() = Some((width, height));
    }
}

// Dropping the player drops the channel's sender; the worker sees the
// disconnect and shuts the engine down on its own thread.

/// Receives the media engine's events on its own threads and forwards them.
#[implement(IMFMediaEngineNotify)]
struct Notify {
    events: Mutex<Sender<i32>>,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _param1: usize, _param2: u32) -> windows::core::Result<()> {
        let _ = self.events.lock().unwrap().send(event as i32);
        Ok(())
    }
}

fn worker(sources: Vec<MediaSource>, options: MediaOptions, shared: Arc<Shared>, commands: Receiver<Command>) {
    // SAFETY: COM is initialised for this thread alone and torn down below.
    let initialised = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    if let Err(error) = run(sources, options, &shared, &commands) {
        eprintln!("[MEDIA] playback failed: {error}");
        shared.status.lock().unwrap().error = true;
        shared.shell().request_paint();
    }
    if initialised {
        unsafe { CoUninitialize() };
    }
}

/// A D3D11 device the media engine can decode into: hardware if there is one,
/// otherwise the WARP software rasterizer.
fn create_device() -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let flags = D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT;
    let mut last_error = None;
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        match try_create_device(driver, flags) {
            Ok(pair) => return Ok(pair),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap())
}

fn try_create_device(
    driver: D3D_DRIVER_TYPE,
    flags: D3D11_CREATE_DEVICE_FLAG,
) -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            flags,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    Ok((device.unwrap(), context.unwrap()))
}

/// The textures one frame passes through: the engine renders into `target`,
/// which is copied into `staging` to be read back.
struct FrameTextures {
    width: u32,
    height: u32,
    target: ID3D11Texture2D,
    staging: ID3D11Texture2D,
}

impl FrameTextures {
    fn new(device: &ID3D11Device, width: u32, height: u32) -> windows::core::Result<Self> {
        let mut desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut target = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut target))? };
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        let mut staging = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging))? };
        Ok(Self { width, height, target: target.unwrap(), staging: staging.unwrap() })
    }
}

fn run(
    sources: Vec<MediaSource>,
    options: MediaOptions,
    shared: &Shared,
    commands: &Receiver<Command>,
) -> windows::core::Result<()> {
    static STARTUP: Once = Once::new();
    STARTUP.call_once(|| {
        // Never shut down: players come and go for the life of the process.
        if let Err(error) = unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) } {
            eprintln!("[MEDIA] MFStartup failed: {error}");
        }
    });

    let (device, context) = create_device()?;
    // The engine decodes on its own threads into this device.
    let multithread: ID3D11Multithread = device.cast()?;
    // Returns the previous setting, which does not matter here.
    let _ = unsafe { multithread.SetMultithreadProtected(true) };
    let mut reset_token = 0u32;
    let mut manager = None;
    unsafe { MFCreateDXGIDeviceManager(&mut reset_token, &mut manager)? };
    let manager = manager.unwrap();
    unsafe { manager.ResetDevice(&device, reset_token)? };

    let (event_tx, events) = mpsc::channel::<i32>();
    let notify: IMFMediaEngineNotify = Notify { events: Mutex::new(event_tx) }.into();

    let mut attributes = None;
    unsafe { MFCreateAttributes(&mut attributes, 3)? };
    let attributes = attributes.unwrap();
    unsafe {
        attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)?;
        attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
        attributes.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32)?;
    }
    let factory: IMFMediaEngineClassFactory =
        unsafe { CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)? };
    let engine: IMFMediaEngine = unsafe { factory.CreateInstance(0, &attributes)? };

    let result = play(&engine, &device, &context, sources, options, shared, commands, &events);
    unsafe {
        let _ = engine.Shutdown();
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn play(
    engine: &IMFMediaEngine,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    sources: Vec<MediaSource>,
    options: MediaOptions,
    shared: &Shared,
    commands: &Receiver<Command>,
    events: &Receiver<i32>,
) -> windows::core::Result<()> {
    unsafe {
        engine.SetMuted(options.muted)?;
        engine.SetLoop(options.looping)?;
    }

    // Offer the sources in order, skipping any whose type the system says it
    // cannot play -- Wikipedia lists a VP8 original first, then VP9 and MPEG-4
    // transcodes -- and falling through to the next on a load error.
    let mut candidates: VecDeque<MediaSource> = sources
        .into_iter()
        .filter(|source| match &source.mime_type {
            Some(mime) => unsafe { engine.CanPlayType(&BSTR::from(mime.as_str())) }
                .map(|can| can != MF_MEDIA_ENGINE_CANPLAY_NOT_SUPPORTED)
                .unwrap_or(true),
            None => true,
        })
        .collect();
    let start_next = |candidates: &mut VecDeque<MediaSource>| -> windows::core::Result<bool> {
        let Some(source) = candidates.pop_front() else {
            return Ok(false);
        };
        eprintln!("[MEDIA] loading {}", source.url);
        unsafe { engine.SetSource(&BSTR::from(source.url.as_str()))? };
        Ok(true)
    };
    if !start_next(&mut candidates)? {
        return Err(windows::core::Error::new(
            MF_E_UNSUPPORTED_FORMAT,
            "none of the video's sources can be played",
        ));
    }
    // An autoplaying video plays as soon as it can; the engine queues the
    // request until enough has loaded. (A click sends its own `Play`.)
    let mut want_play = options.autoplay;
    if want_play {
        unsafe { engine.Play()? };
    }

    let debug = std::env::var_os("ZIPP_DEBUG_MEDIA").is_some();
    let mut metadata = false;
    // How far playback has got, in seconds: an error before it advances means
    // the source is unusable, not that playback failed.
    let mut played_to = 0.0f64;
    let mut failed = false;
    let mut textures: Option<FrameTextures> = None;
    let mut last_pts: Option<i64> = None;
    let mut last_painted = MediaStatus::default();
    let mut last_paint = Instant::now();

    loop {
        // Commands from the document; a dropped player ends playback.
        loop {
            match commands.try_recv() {
                Ok(Command::Play) => {
                    want_play = true;
                    unsafe { engine.Play()? };
                }
                Ok(Command::Pause) => {
                    want_play = false;
                    unsafe { engine.Pause()? };
                }
                Ok(Command::Seek(seconds)) => unsafe { engine.SetCurrentTime(seconds)? },
                Ok(Command::Mute(muted)) => unsafe { engine.SetMuted(muted)? },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        let mut relayout = false;
        while let Ok(event) = events.try_recv() {
            if debug {
                eprintln!("[MEDIA] event {event} t={:.2}", unsafe { engine.GetCurrentTime() });
            }
            if event == MF_MEDIA_ENGINE_EVENT_ERROR.0
                && let Ok(error) = unsafe { engine.GetError() } {
                    let (code, extended) = unsafe { (error.GetErrorCode(), error.GetExtendedErrorCode()) };
                    eprintln!("[MEDIA] error {code} ({:?})", extended);
                }
            if event == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0 {
                metadata = true;
                relayout = true;
            } else if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 && played_to < 0.5 && !candidates.is_empty() {
                // This source would not load or decode -- Wikipedia's VP8
                // original loads its metadata, then fails to decode -- so try
                // the next one, as long as playback never really got going.
                metadata = false;
                last_pts = None;
                *shared.frame.lock().unwrap() = None;
                if !start_next(&mut candidates)? {
                    return Err(windows::core::Error::new(
                        MF_E_UNSUPPORTED_FORMAT,
                        "none of the video's sources could be loaded",
                    ));
                }
                if want_play {
                    unsafe { engine.Play()? };
                }
            } else if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 {
                eprintln!("[MEDIA] playback failed and no other source is left");
                failed = true;
            }
        }

        // A new frame, if the engine has one.
        let mut new_frame = false;
        if metadata && unsafe { engine.HasVideo() }.as_bool()
            && let Ok(pts) = unsafe { engine.OnVideoStreamTick() }
                && last_pts != Some(pts) {
                    last_pts = Some(pts);
                    let display_size = *shared.display_size.lock().unwrap();
                    if let Some(frame) = read_frame(engine, device, context, &mut textures, display_size)? {
                        *shared.frame.lock().unwrap() = Some(frame);
                        new_frame = true;
                    }
                }

        // Publish the state.
        let status = unsafe {
            let mut width = 0u32;
            let mut height = 0u32;
            let size = (metadata
                && engine.GetNativeVideoSize(Some(&mut width), Some(&mut height)).is_ok()
                && width > 0
                && height > 0)
                .then_some((width, height));
            let duration = engine.GetDuration();
            MediaStatus {
                paused: engine.IsPaused().as_bool(),
                ended: engine.IsEnded().as_bool(),
                muted: engine.GetMuted().as_bool(),
                position: engine.GetCurrentTime(),
                duration: (duration.is_finite() && duration > 0.0).then_some(duration),
                video_size: size,
                error: failed,
            }
        };
        *shared.status.lock().unwrap() = status;
        played_to = played_to.max(status.position);

        // Ask for the window to be drawn again: laid out afresh when the size
        // became known, otherwise only repainted -- for a frame, or for the
        // controls (paused/playing, muted, and the progress bar about twice a
        // second for audio-only or stalled video).
        let shell = shared.shell();
        if relayout {
            shell.request_redraw();
        } else if new_frame
            || status.paused != last_painted.paused
            || status.ended != last_painted.ended
            || status.muted != last_painted.muted
            || status.duration != last_painted.duration
            || last_paint.elapsed() > Duration::from_millis(500) && status.position != last_painted.position
        {
            shell.request_paint();
            last_painted = status;
            last_paint = Instant::now();
        }

        let playing = !status.paused && !status.ended;
        std::thread::sleep(Duration::from_millis(if playing { 8 } else { 40 }));
    }
}

/// Copy the engine's current frame out as an RGBA image.
fn read_frame(
    engine: &IMFMediaEngine,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    textures: &mut Option<FrameTextures>,
    display_size: Option<(u32, u32)>,
) -> windows::core::Result<Option<RasterImageData>> {
    let (mut width, mut height) = (0u32, 0u32);
    unsafe { engine.GetNativeVideoSize(Some(&mut width), Some(&mut height))? };
    if width == 0 || height == 0 {
        return Ok(None);
    }
    // Copy out no more pixels than are painted: the engine scales the frame
    // down on the GPU as it transfers it. Wikipedia shows a 2558x1626 original
    // in a 250px box; copying it in full -- 9MB a frame, then scaled down again
    // by the painter -- cost over half a core.
    let mut limit = MAX_FRAME_WIDTH;
    if let Some((display_width, display_height)) = display_size.filter(|&(w, h)| w > 0 && h > 0) {
        // Fit within the display box, keeping the video's aspect ratio.
        let fit_width = (display_height as u64 * width as u64 / height as u64) as u32;
        let floor = (width / MAX_ENGINE_DOWNSCALE).max(1);
        limit = limit.min(display_width.min(fit_width).max(floor));
    }
    if width > limit {
        height = (height as u64 * limit as u64 / width as u64).max(1) as u32;
        width = limit;
    }
    if textures.as_ref().is_none_or(|t| t.width != width || t.height != height) {
        *textures = Some(FrameTextures::new(device, width, height)?);
    }
    let textures = textures.as_ref().unwrap();

    let dest = RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };
    unsafe {
        engine.TransferVideoFrame(&textures.target, None, &dest, None)?;
        context.CopyResource(&textures.staging, &textures.target);
    }
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(&textures.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))? };
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    let row_bytes = width as usize * 4;
    for y in 0..height as usize {
        // SAFETY: the mapped staging texture is `height` rows of `RowPitch`
        // bytes, each holding at least `width` BGRA pixels.
        let src = unsafe {
            std::slice::from_raw_parts((mapped.pData as *const u8).add(y * mapped.RowPitch as usize), row_bytes)
        };
        let dst = &mut rgba[y * row_bytes..(y + 1) * row_bytes];
        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = 255;
        }
    }
    unsafe { context.Unmap(&textures.staging, 0) };
    Ok(Some(RasterImageData::new(width, height, Arc::new(rgba))))
}
