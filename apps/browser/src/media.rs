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

use windows::Win32::Foundation::{GlobalFree, HGLOBAL, HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::{
    CreateStreamOnHGlobal, PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
    PropVariantClear, PropVariantToGUID, PropVariantToInt64,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Variant::VT_I8;
use windows::core::{BSTR, GUID, HSTRING, Interface};
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
        Self { force_muted: false }
    }

    /// Provider that always starts players muted (e.g. for headless `--screenshot` video checks).
    pub fn muted() -> Self {
        Self { force_muted: true }
    }
}

impl MediaProvider for MediaFoundationProvider {
    fn create_player(
        &self,
        sources: Vec<MediaSource>,
        mut options: MediaOptions,
        shell: Arc<dyn ShellProvider>,
    ) -> Option<Arc<dyn MediaPlayer>> {
        let force_muted = self.force_muted;
        options.muted |= force_muted || crate::settings::is_media_muted();
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
            .spawn(move || worker(sources, options, force_muted, worker_shared, rx))
            .ok()?;
        Some(Arc::new(Player { commands: Mutex::new(tx), shared }))
    }
}

enum Command {
    Play,
    Pause,
    Seek(f64),
    Mute(bool),
    Volume(f64),
    PlaybackRate(f64),
    Loop(bool),
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
        self.shell.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

struct Player {
    commands: Mutex<Sender<Command>>,
    shared: Arc<Shared>,
}

impl Player {
    fn send(&self, command: Command) {
        let _ = self.commands.lock().unwrap_or_else(|e| e.into_inner()).send(command);
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
        self.shared.status.lock().unwrap_or_else(|e| e.into_inner()).muted = muted;
        self.send(Command::Mute(muted));
    }
    fn set_volume(&self, volume: f64) {
        let v = volume.clamp(0.0, 1.0);
        self.shared.status.lock().unwrap_or_else(|e| e.into_inner()).volume = v;
        self.send(Command::Volume(v));
    }
    fn set_playback_rate(&self, rate: f64) {
        let r = rate.clamp(0.25, 4.0);
        self.shared.status.lock().unwrap_or_else(|e| e.into_inner()).playback_rate = r;
        self.send(Command::PlaybackRate(r));
    }
    fn set_loop(&self, looping: bool) {
        self.send(Command::Loop(looping));
    }
    fn status(&self) -> MediaStatus {
        *self.shared.status.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn frame(&self) -> Option<RasterImageData> {
        self.shared.frame.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn set_shell_provider(&self, shell: Arc<dyn ShellProvider>) {
        *self.shared.shell.lock().unwrap_or_else(|e| e.into_inner()) = shell;
    }
    fn set_display_size(&self, width: u32, height: u32) {
        *self.shared.display_size.lock().unwrap_or_else(|e| e.into_inner()) = Some((width, height));
    }
}

// Dropping the player drops the channel's sender; the worker sees the
// disconnect and shuts the engine down on its own thread.

/// Receives the media engine's events on its own threads and forwards them.
#[implement(IMFMediaEngineNotify)]
struct Notify {
    events: Mutex<Sender<(i32, usize, u32)>>,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, param1: usize, param2: u32) -> windows::core::Result<()> {
        let _ = self
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send((event as i32, param1, param2));
        Ok(())
    }
}

fn worker(
    sources: Vec<MediaSource>,
    options: MediaOptions,
    force_muted: bool,
    shared: Arc<Shared>,
    commands: Receiver<Command>,
) {
    // SAFETY: COM is initialised for this thread alone and torn down below.
    let initialised = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    if let Err(error) = run(sources, options, force_muted, &shared, &commands) {
        eprintln!("[MEDIA] playback failed: {error}");
        shared.status.lock().unwrap_or_else(|e| e.into_inner()).error = true;
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
    force_muted: bool,
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

    // When no active Windows audio endpoint exists (headless/RDP/unplugged
    // speakers) or in forced-mute `--screenshot` mode, `IMFMediaEngine`'s
    // Streaming Audio Renderer fails with `MF_E_NO_AUDIO_PLAYBACK_DEVICE` and
    // stalls the video presentation clock. Use `IMFSourceReader` directly to
    // decode and pace video frames with zero audio-device dependency.
    let no_audio_renderer = force_muted || !crate::audio::has_output_endpoint();
    if no_audio_renderer {
        return play_source_reader(&sources, options, shared, commands);
    }

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

    let (event_tx, events) = mpsc::channel::<(i32, usize, u32)>();
    let notify: IMFMediaEngineNotify = Notify { events: Mutex::new(event_tx) }.into();

    let mut attributes = None;
    unsafe { MFCreateAttributes(&mut attributes, 4)? };
    let attributes = attributes.unwrap();
    unsafe {
        attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)?;
        attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
        attributes.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32)?;
        let _ = attributes.SetUINT32(&MF_MEDIA_ENGINE_CONTINUE_ON_CODEC_ERROR, 1);
    }
    let factory: IMFMediaEngineClassFactory =
        unsafe { CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)? };
    let engine: IMFMediaEngine = unsafe { factory.CreateInstance(0, &attributes)? };

    let result = play(
        &engine,
        &device,
        &context,
        sources.clone(),
        options,
        false,
        shared,
        commands,
        &events,
    );
    unsafe {
        let _ = engine.Shutdown();
    }
    // If speakers were unplugged mid-init (`MF_E_NO_AUDIO_PLAYBACK_DEVICE`),
    // fall back seamlessly to `IMFSourceReader` so video still plays.
    if matches!(&result, Err(e) if e.code() == windows::core::HRESULT(0xC00D4E84u32 as i32)) {
        eprintln!("[MEDIA] audio device unavailable during engine start, falling back to video-only reader");
        return play_source_reader(&sources, options, shared, commands);
    }
    result
}

/// Deselect audio tracks via `IMFMediaEngineEx` when the Streaming Audio
/// Renderer fails to initialize.
fn deselect_audio_streams(engine: &IMFMediaEngine, debug: bool) -> bool {
    let Ok(engine_ex) = engine.cast::<IMFMediaEngineEx>() else {
        return false;
    };
    let Ok(count) = (unsafe { engine_ex.GetNumberOfStreams() }) else {
        return false;
    };
    let mut changed = false;
    for idx in 0..count {
        let is_audio = unsafe { engine_ex.GetStreamAttribute(idx, &MF_MT_MAJOR_TYPE) }
            .ok()
            .and_then(|mut pv| {
                let g = unsafe { PropVariantToGUID(&pv).ok() };
                let _ = unsafe { PropVariantClear(&mut pv) };
                g
            })
            .is_some_and(|guid| guid == MFMediaType_Audio);
        if debug {
            eprintln!("[MEDIA] stream {idx}/{count}: is_audio={is_audio}");
        }
        if is_audio && unsafe { engine_ex.SetStreamSelection(idx, false) }.is_ok() {
            changed = true;
        }
    }
    if changed {
        if let Err(e) = unsafe { engine_ex.ApplyStreamSelections() } {
            eprintln!("[MEDIA] ApplyStreamSelections failed: {e}");
            return false;
        }
        if debug {
            eprintln!("[MEDIA] deselected audio stream(s) to unblock video rendering");
        }
    }
    changed
}

fn propvariant_i64(val: i64) -> PROPVARIANT {
    PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_I8,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 { hVal: val },
            }),
        },
    }
}

/// Build an in-memory `IMFByteStream` backed by `GlobalAlloc` (zero temp files,
/// Incognito-safe) along with a container extension hint and MIME content type.
fn create_mf_byte_stream(bytes: &[u8], mime: &str) -> windows::core::Result<(IMFByteStream, &'static str)> {
    if bytes.is_empty() {
        return Err(windows::core::Error::new(MF_E_UNSUPPORTED_FORMAT, "empty media bytes"));
    }
    let hglobal: HGLOBAL = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len())? };
    let ptr = unsafe { GlobalLock(hglobal) };
    if ptr.is_null() {
        let _ = unsafe { GlobalFree(Some(hglobal)) };
        return Err(windows::core::Error::from_thread());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(hglobal);
    }
    // `fDeleteOnRelease = true` transfers ownership of `hglobal` to the COM stream.
    let stream = unsafe { CreateStreamOnHGlobal(hglobal, true)? };
    let mf_stream = unsafe { MFCreateMFByteStreamOnStream(&stream)? };
    let (ext, content_type) = if mime.contains("webm") || bytes.starts_with(b"\x1A\x45\xDF\xA3") {
        ("webm", "video/webm")
    } else if mime.contains("quicktime") || mime.contains("mov") {
        ("mov", "video/quicktime")
    } else if mime.contains("ogg") || bytes.starts_with(b"OggS") {
        ("ogv", "video/ogg")
    } else {
        ("mp4", "video/mp4")
    };
    if let Ok(attrs) = mf_stream.cast::<IMFAttributes>() {
        let _ = unsafe { attrs.SetString(&MF_BYTESTREAM_CONTENT_TYPE, &HSTRING::from(content_type)) };
    }
    Ok((mf_stream, ext))
}

/// Load an in-memory byte slice into Media Foundation without touching disk
/// (supports `blob:`, `data:`, and fetched HTTP videos cleanly, including in Incognito mode).
fn set_source_bytes(engine: &IMFMediaEngine, bytes: &[u8], mime: &str) -> windows::core::Result<()> {
    let engine_ex: IMFMediaEngineEx = engine.cast()?;
    let (mf_stream, ext) = create_mf_byte_stream(bytes, mime)?;
    let hint = format!("memory://media.{ext}");
    unsafe { engine_ex.SetSourceFromByteStream(&mf_stream, &BSTR::from(hint.as_str()))? };
    Ok(())
}

/// Open an `IMFSourceReader` for `source` with video processing enabled (converts
/// YUV/NV12 to RGB32 cleanly without requiring any audio playback hardware).
fn open_source_reader(source: &MediaSource) -> windows::core::Result<IMFSourceReader> {
    let mut attrs = None;
    unsafe { MFCreateAttributes(&mut attrs, 1)? };
    let attrs = attrs.unwrap();
    unsafe { attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)? };

    let url = source.url.as_str();
    let in_mem: Option<(String, Vec<u8>)> = if url.starts_with("blob:") {
        crate::lookup_blob(url)
    } else if url.starts_with("data:") {
        crate::decode_data_url(url)
    } else if url.starts_with("http://") || url.starts_with("https://") {
        crate::fetch_media_bytes(url)
    } else {
        None
    };

    if let Some((mime, bytes)) = in_mem {
        let eff_mime = if !mime.is_empty() {
            mime
        } else {
            source.mime_type.clone().unwrap_or_default()
        };
        eprintln!("[MEDIA] source-reader loading {} bytes ({}) for {url}", bytes.len(), eff_mime);
        let (mf_stream, _) = create_mf_byte_stream(&bytes, &eff_mime)?;
        return unsafe { MFCreateSourceReaderFromByteStream(&mf_stream, &attrs) };
    }
    if url.starts_with("blob:") || url.starts_with("data:") {
        return Err(windows::core::Error::new(MF_E_UNSUPPORTED_FORMAT, "media URL unavailable"));
    }
    let path_or_url = url::Url::parse(url)
        .ok()
        .filter(|u| u.scheme() == "file")
        .and_then(|u| u.to_file_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| url.to_string());
    eprintln!("[MEDIA] source-reader opening {path_or_url}");
    unsafe { MFCreateSourceReaderFromURL(&HSTRING::from(path_or_url.as_str()), &attrs) }
}

/// Decode one RGB32 video frame from `reader`, returning `(timestamp_sec, rgba_image)`.
fn read_source_reader_frame(
    reader: &IMFSourceReader,
    width: u32,
    height: u32,
    stride: i32,
) -> windows::core::Result<Option<(f64, RasterImageData)>> {
    let stream_idx = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    loop {
        let mut flags = 0u32;
        let mut ts_hns = 0i64;
        let mut sample: Option<IMFSample> = None;
        unsafe {
            reader.ReadSample(
                stream_idx,
                0,
                None,
                Some(&mut flags),
                Some(&mut ts_hns),
                Some(&mut sample),
            )?;
        }
        if (flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 {
            return Ok(None);
        }
        let Some(sample) = sample else {
            continue;
        };
        let buffer = unsafe { sample.ConvertToContiguousBuffer()? };
        let mut ptr: *mut u8 = std::ptr::null_mut();
        let mut cur_len = 0u32;
        unsafe { buffer.Lock(&mut ptr, None, Some(&mut cur_len))? };

        let w = width as usize;
        let h = height as usize;
        let row_bytes = w * 4;
        let abs_stride = (stride.unsigned_abs() as usize).max(row_bytes);
        let available = cur_len as usize;
        let mut rgba = vec![0u8; w * h * 4];
        if !ptr.is_null() && available >= row_bytes {
            let src_all = unsafe { std::slice::from_raw_parts(ptr as *const u8, available) };
            let bottom_up = stride < 0;
            for y in 0..h {
                let src_y = if bottom_up { h - 1 - y } else { y };
                let src_off = src_y.saturating_mul(abs_stride);
                if src_off + row_bytes > available {
                    break;
                }
                let src = &src_all[src_off..src_off + row_bytes];
                let dst = &mut rgba[y * row_bytes..(y + 1) * row_bytes];
                for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = 255;
                }
            }
        }
        let _ = unsafe { buffer.Unlock() };
        let pts_sec = (ts_hns.max(0) as f64) / 10_000_000.0;
        return Ok(Some((pts_sec, RasterImageData::new(width, height, Arc::new(rgba)))));
    }
}

/// Hardware/software Media Foundation video playback via `IMFSourceReader`.
/// Works cleanly with zero audio-device dependency (ideal for headless/RDP,
/// unplugged speakers, and muted `--screenshot` captures).
fn play_source_reader(
    sources: &[MediaSource],
    options: MediaOptions,
    shared: &Shared,
    commands: &Receiver<Command>,
) -> windows::core::Result<()> {
    let mut opened = None;
    let mut last_err = None;
    for source in sources {
        match open_source_reader(source) {
            Ok(reader) => {
                opened = Some(reader);
                break;
            }
            Err(e) => {
                eprintln!("[MEDIA] source-reader skipping {}: {e}", source.url);
                last_err = Some(e);
            }
        }
    }
    let reader = match opened {
        Some(r) => r,
        None => {
            return Err(last_err.unwrap_or_else(|| {
                windows::core::Error::new(MF_E_UNSUPPORTED_FORMAT, "no playable video source")
            }));
        }
    };

    let all_streams = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
    let video_stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    let media_src = MF_SOURCE_READER_MEDIASOURCE.0 as u32;
    unsafe {
        let _ = reader.SetStreamSelection(all_streams, false);
        reader.SetStreamSelection(video_stream, true)?;
        let out_type = MFCreateMediaType()?;
        out_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        out_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        reader.SetCurrentMediaType(video_stream, None, &out_type)?;
    }

    let cur_type = unsafe { reader.GetCurrentMediaType(video_stream)? };
    let packed_size = unsafe { cur_type.GetUINT64(&MF_MT_FRAME_SIZE)? };
    let width = (packed_size >> 32) as u32;
    let height = (packed_size & 0xFFFF_FFFF) as u32;
    if width == 0 || height == 0 {
        return Err(windows::core::Error::new(MF_E_UNSUPPORTED_FORMAT, "zero video dimensions"));
    }
    let stride = unsafe { cur_type.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
        .map(|u| u as i32)
        .unwrap_or((width * 4) as i32);

    let duration = unsafe { reader.GetPresentationAttribute(media_src, &MF_PD_DURATION) }
        .ok()
        .and_then(|mut pv| {
            let hns = unsafe { PropVariantToInt64(&pv).ok() };
            let _ = unsafe { PropVariantClear(&mut pv) };
            hns
        })
        .filter(|&hns| hns > 0)
        .map(|hns| hns as f64 / 10_000_000.0);

    let mut paused = !options.autoplay;
    let mut ended = false;
    let mut muted = options.muted;
    let mut volume = 1.0f64;
    let mut playback_rate = 1.0f64;
    let mut looping = options.looping;
    let mut position = 0.0f64;

    // Decode Frame 0 immediately so both autoplaying and paused `<video controls>`
    // paint crisp decoded video pixels right away.
    if let Ok(Some((pts, first_frame))) = read_source_reader_frame(&reader, width, height, stride) {
        position = pts;
        *shared.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(first_frame);
    }

    let publish = |paused: bool,
                   ended: bool,
                   muted: bool,
                   volume: f64,
                   playback_rate: f64,
                   position: f64| {
        *shared.status.lock().unwrap_or_else(|e| e.into_inner()) = MediaStatus {
            paused,
            ended,
            muted,
            volume,
            playback_rate,
            ready_state: 4,
            position,
            duration,
            video_size: Some((width, height)),
            error: false,
        };
    };

    publish(paused, ended, muted, volume, playback_rate, position);
    shared.shell().request_redraw();

    let mut clock_origin = Instant::now();
    let mut clock_base_pts = position;
    let mut pending_frame: Option<(f64, RasterImageData)> = None;

    loop {
        let mut status_dirty = false;
        loop {
            match commands.try_recv() {
                Ok(Command::Play) => {
                    if ended {
                        let pv = propvariant_i64(0);
                        let _ = unsafe { reader.SetCurrentPosition(&GUID::zeroed(), &pv) };
                        position = 0.0;
                        ended = false;
                        pending_frame = None;
                    }
                    paused = false;
                    clock_origin = Instant::now();
                    clock_base_pts = position;
                    status_dirty = true;
                }
                Ok(Command::Pause) => {
                    paused = true;
                    status_dirty = true;
                }
                Ok(Command::Seek(seconds)) => {
                    let target = seconds.max(0.0);
                    let pv = propvariant_i64((target * 10_000_000.0) as i64);
                    let _ = unsafe { reader.SetCurrentPosition(&GUID::zeroed(), &pv) };
                    pending_frame = None;
                    ended = false;
                    if let Ok(Some((pts, frame))) = read_source_reader_frame(&reader, width, height, stride) {
                        position = pts;
                        *shared.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(frame);
                    } else {
                        position = target;
                    }
                    clock_origin = Instant::now();
                    clock_base_pts = position;
                    status_dirty = true;
                }
                Ok(Command::Mute(m)) => {
                    muted = m;
                    status_dirty = true;
                }
                Ok(Command::Volume(v)) => {
                    volume = v.clamp(0.0, 1.0);
                    status_dirty = true;
                }
                Ok(Command::PlaybackRate(r)) => {
                    playback_rate = r.clamp(0.25, 4.0);
                    clock_origin = Instant::now();
                    clock_base_pts = position;
                    status_dirty = true;
                }
                Ok(Command::Loop(l)) => {
                    looping = l;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        let mut new_frame = false;
        if !paused && !ended {
            let media_now = clock_base_pts + clock_origin.elapsed().as_secs_f64() * playback_rate;
            if pending_frame.is_none() {
                match read_source_reader_frame(&reader, width, height, stride) {
                    Ok(Some(f)) => pending_frame = Some(f),
                    Ok(None) => {
                        if looping {
                            let pv = propvariant_i64(0);
                            let _ = unsafe { reader.SetCurrentPosition(&GUID::zeroed(), &pv) };
                            position = 0.0;
                            clock_origin = Instant::now();
                            clock_base_pts = 0.0;
                        } else {
                            ended = true;
                            paused = true;
                            status_dirty = true;
                        }
                    }
                    Err(e) => {
                        eprintln!("[MEDIA] source-reader decode error: {e}");
                        ended = true;
                        paused = true;
                        status_dirty = true;
                    }
                }
            }
            if let Some((pts, _)) = &pending_frame
                && *pts <= media_now + 0.004
            {
                let (pts, frame) = pending_frame.take().unwrap();
                position = pts;
                *shared.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(frame);
                new_frame = true;
            }
        }

        if new_frame || status_dirty {
            publish(paused, ended, muted, volume, playback_rate, position);
            shared.shell().request_paint();
        }

        std::thread::sleep(Duration::from_millis(if !paused && !ended { 8 } else { 40 }));
    }
}

fn load_source(engine: &IMFMediaEngine, source: &MediaSource) -> windows::core::Result<()> {
    let url = source.url.as_str();
    if url.starts_with("blob:") {
        if let Some((mime, bytes)) = crate::lookup_blob(url) {
            let eff_mime = if !mime.is_empty() {
                mime
            } else {
                source.mime_type.clone().unwrap_or_default()
            };
            eprintln!("[MEDIA] loading in-memory blob ({} bytes, {})", bytes.len(), eff_mime);
            return set_source_bytes(engine, &bytes, &eff_mime);
        }
        return Err(windows::core::Error::new(MF_E_UNSUPPORTED_FORMAT, "blob URL not found"));
    }
    if url.starts_with("data:") {
        if let Some((mime, bytes)) = crate::decode_data_url(url) {
            eprintln!("[MEDIA] loading in-memory data URL ({} bytes, {})", bytes.len(), mime);
            return set_source_bytes(engine, &bytes, &mime);
        }
        return Err(windows::core::Error::new(MF_E_UNSUPPORTED_FORMAT, "invalid data URL"));
    }
    if (url.starts_with("http://") || url.starts_with("https://"))
        && let Some((mime, bytes)) = crate::fetch_media_bytes(url)
    {
        let eff_mime = if !mime.is_empty() {
            mime
        } else {
            source.mime_type.clone().unwrap_or_default()
        };
        eprintln!("[MEDIA] loading fetched HTTP media ({} bytes, {}) from {url}", bytes.len(), eff_mime);
        return set_source_bytes(engine, &bytes, &eff_mime);
    }
    eprintln!("[MEDIA] loading {url}");
    unsafe { engine.SetSource(&BSTR::from(url))? };
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn play(
    engine: &IMFMediaEngine,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    sources: Vec<MediaSource>,
    options: MediaOptions,
    no_audio_renderer: bool,
    shared: &Shared,
    commands: &Receiver<Command>,
    events: &Receiver<(i32, usize, u32)>,
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
        .filter(|source| match source.mime_type.as_deref() {
            Some(mime) if !mime.is_empty() => unsafe { engine.CanPlayType(&BSTR::from(mime)) }
                .map(|can| can != MF_MEDIA_ENGINE_CANPLAY_NOT_SUPPORTED)
                .unwrap_or(true),
            _ => true,
        })
        .collect();
    let start_next = |candidates: &mut VecDeque<MediaSource>| -> windows::core::Result<bool> {
        while let Some(source) = candidates.pop_front() {
            match load_source(engine, &source) {
                Ok(()) => return Ok(true),
                Err(e) => {
                    eprintln!("[MEDIA] skipping source {}: {e}", source.url);
                }
            }
        }
        Ok(false)
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
    let mut audio_deselected = false;
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
                Ok(Command::Seek(seconds)) => {
                    let _ = unsafe { engine.SetCurrentTime(seconds.max(0.0)) };
                    last_pts = None;
                }
                Ok(Command::Mute(muted)) => {
                    let _ = unsafe { engine.SetMuted(muted) };
                }
                Ok(Command::Volume(vol)) => {
                    let _ = unsafe { engine.SetVolume(vol.clamp(0.0, 1.0)) };
                }
                Ok(Command::PlaybackRate(rate)) => {
                    let _ = unsafe { engine.SetPlaybackRate(rate.clamp(0.25, 4.0)) };
                }
                Ok(Command::Loop(looping)) => {
                    let _ = unsafe { engine.SetLoop(looping) };
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        let mut relayout = false;
        while let Ok((event, param1, param2)) = events.try_recv() {
            if debug {
                eprintln!(
                    "[MEDIA] event {event} p1={param1} p2=0x{param2:08x} t={:.2}",
                    unsafe { engine.GetCurrentTime() }
                );
            }
            if event == MF_MEDIA_ENGINE_EVENT_ERROR.0
                && let Ok(error) = unsafe { engine.GetError() }
            {
                let (code, extended) = unsafe { (error.GetErrorCode(), error.GetExtendedErrorCode()) };
                eprintln!("[MEDIA] error {code} ({:?})", extended);
            }
            if event == MF_MEDIA_ENGINE_EVENT_STREAMRENDERINGERROR.0
                && param2 == MF_MEDIA_ENGINE_STREAMTYPE_FAILED_AUDIO.0 as u32
            {
                if (param1 as u32) == 0xC00D4E84u32 {
                    return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                        0xC00D4E84u32 as i32,
                    )));
                }
                if !audio_deselected && deselect_audio_streams(engine, debug) {
                    audio_deselected = true;
                    last_pts = None;
                    if want_play {
                        let _ = unsafe { engine.Play() };
                    }
                }
            }
            if event == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0
                || event == MF_MEDIA_ENGINE_EVENT_LOADEDDATA.0
                || event == MF_MEDIA_ENGINE_EVENT_CANPLAY.0
                || event == MF_MEDIA_ENGINE_EVENT_CANPLAYTHROUGH.0
                || event == MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY.0
            {
                if !metadata {
                    relayout = true;
                }
                if no_audio_renderer && !audio_deselected && deselect_audio_streams(engine, debug) {
                    audio_deselected = true;
                    last_pts = None;
                }
                if event == MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY.0 {
                    last_pts = None;
                }
                metadata = true;
                if want_play && unsafe { engine.IsPaused() }.as_bool() {
                    let _ = unsafe { engine.Play() };
                }
            } else if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 && played_to < 0.5 && !candidates.is_empty() {
                // This source would not load or decode -- Wikipedia's VP8
                // original loads its metadata, then fails to decode -- so try
                // the next one, as long as playback never really got going.
                metadata = false;
                audio_deselected = false;
                last_pts = None;
                *shared.frame.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
        if metadata
            && unsafe { engine.HasVideo() }.as_bool()
            && let Ok(pts) = unsafe { engine.OnVideoStreamTick() }
            && last_pts != Some(pts)
        {
            last_pts = Some(pts);
            let display_size = *shared.display_size.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(frame) = read_frame(engine, device, context, &mut textures, display_size)? {
                *shared.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(frame);
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
            let vol = engine.GetVolume();
            let rate = engine.GetPlaybackRate();
            let rs = engine.GetReadyState().min(4) as u8;
            MediaStatus {
                paused: engine.IsPaused().as_bool(),
                ended: engine.IsEnded().as_bool(),
                muted: engine.GetMuted().as_bool(),
                volume: if vol.is_finite() { vol.clamp(0.0, 1.0) } else { 1.0 },
                playback_rate: if rate.is_finite() && rate > 0.0 { rate } else { 1.0 },
                ready_state: rs,
                position: engine.GetCurrentTime(),
                duration: (duration.is_finite() && duration > 0.0).then_some(duration),
                video_size: size,
                error: failed,
            }
        };
        *shared.status.lock().unwrap_or_else(|e| e.into_inner()) = status;
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
