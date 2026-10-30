//! Sound. A page's Web Audio (`webaudio_shim.js`) is mixed in JavaScript and
//! played here; the files it decodes (`decodeAudioData`, `<audio>`) are
//! decoded here.
//!
//! The page writes its frames a little ahead of time: 32-bit float,
//! interleaved stereo, at the rate `open` answered. An output thread plays
//! them -- through ALSA's default device on Linux -- and counts the frames
//! that have reached the speaker, which is the page's
//! `AudioContext.currentTime`. When the page has written nothing it plays
//! silence, so that time goes on. Without a sound card (or off Linux) the
//! thread keeps the same time against the clock and plays nothing.
//!
//! Files are decoded with symphonia: MP3, Ogg Vorbis, FLAC and WAV.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use zipp_vm::embed::HostCtx;

/// Frames per write to the device: about 10 ms.
const PERIOD: usize = 480;

struct Output {
    rate: u32,
    /// Interleaved stereo samples the page has written, not yet played.
    queue: Mutex<VecDeque<f32>>,
    /// Frames that have reached the speaker since the output opened.
    played: AtomicU64,
}

static OUTPUT: OnceLock<Arc<Output>> = OnceLock::new();
static DECODED: Mutex<Option<HashMap<u32, Decoded>>> = Mutex::new(None);
static NEXT_DECODED: AtomicU64 = AtomicU64::new(1);

struct Decoded {
    rate: u32,
    channels: Vec<Vec<f32>>,
}

/// `__zippHostCall("audio", op, ...)`.
pub(crate) fn host_call(ctx: &mut dyn HostCtx, args: &[String]) -> Result<String, String> {
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
    let num = |i: usize| arg(i).parse::<usize>().map_err(|_| format!("Error: audio: argument {i} is not a number"));
    match arg(0) {
        "open" => Ok(format!("{{\"sampleRate\":{}}}", output().rate)),
        "write" => {
            let bytes = page_bytes(ctx, num(1)?)?;
            let out = output();
            let mut queue = out.queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.extend(bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
            Ok(String::new())
        }
        "time" => Ok(output().played.load(Ordering::Acquire).to_string()),
        "decode" => {
            let bytes = page_bytes(ctx, num(1)?)?.to_vec();
            let decoded = decode(bytes)?;
            let id = NEXT_DECODED.fetch_add(1, Ordering::Relaxed) as u32;
            let info = format!(
                "{{\"id\":{id},\"sampleRate\":{},\"channels\":{},\"frames\":{}}}",
                decoded.rate,
                decoded.channels.len(),
                decoded.channels.first().map_or(0, Vec::len)
            );
            DECODED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(id, decoded);
            Ok(info)
        }
        // The decoded samples, planar, into the page's bytes (sized for them).
        "take" => {
            let id = num(1)? as u32;
            let decoded = DECODED
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
                .and_then(|d| d.remove(&id))
                .ok_or("Error: audio: no such decoded file")?;
            let (addr, count, kind) = ctx.typed_array_region("__zippHostBytes")?;
            let needed: usize = decoded.channels.iter().map(|c| c.len() * 4).sum();
            if kind != 1 || count < needed {
                return Err("Error: audio: __zippHostBytes is too small for the samples".into());
            }
            // SAFETY: the engine pinned `count` bytes at `addr` for this call.
            let out = unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, needed) };
            for (chunk, sample) in out.chunks_exact_mut(4).zip(decoded.channels.iter().flatten()) {
                chunk.copy_from_slice(&sample.to_le_bytes());
            }
            Ok(String::new())
        }
        op => Err(format!("Error: audio: no such operation {op}")),
    }
}

/// The first `len` bytes of the page's `__zippHostBytes`.
fn page_bytes(ctx: &mut dyn HostCtx, len: usize) -> Result<&'static [u8], String> {
    if len == 0 {
        return Ok(&[]);
    }
    let (addr, count, kind) = ctx.typed_array_region("__zippHostBytes")?;
    if kind != 1 || count < len {
        return Err("Error: audio: __zippHostBytes does not hold the bytes".into());
    }
    // SAFETY: the engine pinned `count` bytes at `addr` for this call, and
    // the slice does not outlive it.
    Ok(unsafe { std::slice::from_raw_parts(addr as *const u8, len) })
}

/// The output, opened on first use.
fn output() -> Arc<Output> {
    OUTPUT
        .get_or_init(|| {
            #[cfg(target_os = "linux")]
            match alsa_out::open() {
                Ok((pcm, rate)) => {
                    let out = Arc::new(Output { rate, queue: Mutex::default(), played: AtomicU64::new(0) });
                    let shared = out.clone();
                    let _ = std::thread::Builder::new().name("audio".into()).spawn(move || alsa_out::play(pcm, &shared));
                    eprintln!("[AUDIO] playing at {rate} Hz");
                    return out;
                }
                Err(e) => eprintln!("[AUDIO] no sound output ({e}): keeping time silently"),
            }
            let out = Arc::new(Output { rate: 48_000, queue: Mutex::default(), played: AtomicU64::new(0) });
            let shared = out.clone();
            let _ = std::thread::Builder::new().name("audio".into()).spawn(move || silent(&shared));
            out
        })
        .clone()
}

/// Up to one period of what the page wrote, padded with silence.
fn next_period(out: &Output, buf: &mut [f32]) {
    let mut queue = out.queue.lock().unwrap_or_else(|e| e.into_inner());
    for sample in buf.iter_mut() {
        *sample = queue.pop_front().unwrap_or(0.0);
    }
}

/// No device (or no longer one): consume what the page writes at the rate it
/// would be played, from where the time has got to.
fn silent(out: &Output) {
    let mut buf = vec![0.0f32; PERIOD * 2];
    let start = Instant::now();
    let base = out.played.load(Ordering::Acquire);
    let mut frames: u64 = 0;
    loop {
        next_period(out, &mut buf);
        frames += PERIOD as u64;
        out.played.store(base + frames, Ordering::Release);
        let due = start + Duration::from_secs_f64(frames as f64 / out.rate as f64);
        std::thread::sleep(due.saturating_duration_since(Instant::now()));
    }
}

#[cfg(target_os = "linux")]
mod alsa_out {
    use std::sync::atomic::Ordering;

    use alsa::pcm::{Access, Format, HwParams, PCM};
    use alsa::{Direction, ValueOr};

    use super::{Output, PERIOD};

    /// The first device that opens, stereo float at 48 kHz or what is
    /// nearest: `ZIPP_AUDIO_DEVICE` (an ALSA name, such as `plughw:1,0`),
    /// then every card's playback devices -- speakers and headphones before
    /// HDMI, which is often a monitor without any -- then each card as ALSA's
    /// configuration for it has it (a Raspberry Pi's HDMI takes IEC958
    /// frames, which only that makes), and then ALSA's "default". Each
    /// device is tried through ALSA's mixer (dmix) first: other programs
    /// play on the card at the same time -- a browser in a window of
    /// zipp-linux's desktop, its Linux apps -- where a device opened
    /// directly is one program's until it lets go.
    pub(super) fn open() -> Result<(PCM, u32), String> {
        let mut candidates: Vec<(String, String)> = Vec::new();
        if let Ok(name) = std::env::var("ZIPP_AUDIO_DEVICE") {
            candidates.push((name.clone(), name));
        }
        let mut hdmi = Vec::new();
        let mut cards = Vec::new();
        for card in alsa::card::Iter::new().flatten() {
            let Ok(ctl) = alsa::Ctl::from_card(&card, false) else { continue };
            let card_name = card.get_name().unwrap_or_default();
            // ALSA's virtual cards, which lead nowhere.
            if card_name == "Loopback" || card_name == "Dummy" {
                continue;
            }
            cards.push((format!("sysdefault:{}", card.get_index()), card_name.clone()));
            for device in alsa::ctl::DeviceIter::new(&ctl) {
                let Ok(info) = ctl.pcm_info(device as u32, 0, Direction::Playback) else { continue };
                let label = format!("{card_name}: {}", info.get_name().unwrap_or_default());
                let mixed = format!("plug:\"dmix:CARD={},DEV={device}\"", card.get_index());
                let name = format!("plughw:{},{device}", card.get_index());
                let list = if label.contains("HDMI") || label.contains("DisplayPort") { &mut hdmi } else { &mut candidates };
                list.push((mixed, format!("{label}, mixed")));
                list.push((name, label));
            }
        }
        candidates.extend(hdmi);
        candidates.extend(cards);
        candidates.push(("default".into(), "ALSA's default".into()));
        let mut failures = Vec::new();
        for (name, label) in candidates {
            match open_device(&name) {
                Ok((pcm, rate)) => {
                    for failure in &failures {
                        eprintln!("[AUDIO] passed over {failure}");
                    }
                    eprintln!("[AUDIO] {name} ({label})");
                    unmute(&name);
                    return Ok((pcm, rate));
                }
                Err(e) => failures.push(format!("{name}: {e}")),
            }
        }
        Err(failures.join("; "))
    }

    /// A sound card starts muted (a desktop's `alsactl` unmutes it): turn its
    /// outputs on, at 0 dB where they reach it. Only outputs: a microphone's
    /// or line input's path to the speakers stays off, or it would howl.
    fn unmute(device: &str) {
        use alsa::Round;
        use alsa::mixer::{MilliBel, Mixer, Selem};
        // "plughw:1,0" and `dmix:CARD=1,...` and the like name the card;
        // anything else is card 0.
        let card = match device.find("CARD=") {
            Some(at) => device[at + 5..].split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse::<u32>().ok()),
            None => device.split_once(':').and_then(|(_, rest)| rest.split(',').next()).and_then(|c| c.parse::<u32>().ok()),
        }
        .unwrap_or(0);
        let Ok(mixer) = Mixer::new(&format!("hw:{card}"), false) else { return };
        let mut on = Vec::new();
        for selem in mixer.iter().filter_map(Selem::new) {
            let id = selem.get_id();
            let name = id.get_name().unwrap_or_default();
            let input = ["Mic", "Capture", "Beep", "Loopback", "Aux", "CD", "Phone", "Video", "Line In"]
                .iter()
                .any(|word| name.contains(word))
                || (name.starts_with("Line") && !name.starts_with("Line Out"));
            if input || !selem.can_playback() {
                continue;
            }
            if selem.has_playback_switch() {
                let _ = selem.set_playback_switch_all(1);
            }
            if selem.has_playback_volume() {
                let (min, max) = selem.get_playback_volume_range();
                let (db_min, db_max) = selem.get_playback_db_range();
                if db_min.0 < db_max.0 && db_min.0 <= 0 {
                    let _ = selem.set_playback_db_all(MilliBel(0.min(db_max.0)), Round::Floor);
                } else {
                    let _ = selem.set_playback_volume_all(min + (max - min) * 4 / 5);
                }
            }
            on.push(name.to_string());
        }
        if !on.is_empty() {
            eprintln!("[AUDIO] turned on: {}", on.join(", "));
        }
    }

    fn open_device(name: &str) -> Result<(PCM, u32), String> {
        let pcm = PCM::new(name, Direction::Playback, false).map_err(|e| e.to_string())?;
        let rate = {
            let hw = HwParams::any(&pcm).map_err(|e| e.to_string())?;
            hw.set_channels(2).map_err(|e| e.to_string())?;
            hw.set_rate(48_000, ValueOr::Nearest).map_err(|e| e.to_string())?;
            hw.set_format(Format::float()).map_err(|e| e.to_string())?;
            hw.set_access(Access::RWInterleaved).map_err(|e| e.to_string())?;
            hw.set_period_size_near(PERIOD as alsa::pcm::Frames, ValueOr::Nearest).map_err(|e| e.to_string())?;
            hw.set_buffer_size_near(PERIOD as alsa::pcm::Frames * 6).map_err(|e| e.to_string())?;
            pcm.hw_params(&hw).map_err(|e| e.to_string())?;
            hw.get_rate().map_err(|e| e.to_string())?
        };
        Ok((pcm, rate))
    }

    /// Write period after period, blocking on the device, which paces it.
    pub(super) fn play(pcm: PCM, out: &Output) {
        let io = match pcm.io_f32() {
            Ok(io) => io,
            Err(e) => {
                eprintln!("[AUDIO] {e}: keeping time silently");
                return super::silent(out);
            }
        };
        let mut buf = vec![0.0f32; PERIOD * 2];
        let mut written: u64 = 0;
        let mut delay: u64 = 0;
        loop {
            super::next_period(out, &mut buf);
            match io.writei(&buf) {
                Ok(frames) => written += frames as u64,
                Err(e) => {
                    // An underrun: start again, the same period.
                    if pcm.try_recover(e, true).is_err() {
                        eprintln!("[AUDIO] the device failed ({e}): keeping time silently");
                        return super::silent(out);
                    }
                    continue;
                }
            }
            // Kept from last time when the device cannot say, so that the
            // page's time never runs backwards.
            if let Ok(now) = pcm.delay() {
                delay = now.max(0) as u64;
            }
            let played = written.saturating_sub(delay).max(out.played.load(Ordering::Acquire));
            out.played.store(played, Ordering::Release);
        }
    }
}

/// A whole file, decoded to planar float samples.
fn decode(bytes: Vec<u8>) -> Result<Decoded, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let fail = |e: Error| format!("EncodingError: Unable to decode audio data: {e}");
    let source = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let probed = symphonia::default::get_probe()
        .format(&Hint::new(), source, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(fail)?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("EncodingError: Unable to decode audio data: no audio track")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(fail)?;
    let mut rate = track.codec_params.sample_rate.unwrap_or(48_000);
    let mut channels: Vec<Vec<f32>> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(_)) => break,
            Err(e) => return Err(fail(e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            // A damaged frame is skipped, as players do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(fail(e)),
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        let count = spec.channels.count();
        if channels.len() < count {
            channels.resize(count, Vec::new());
        }
        let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        samples.copy_interleaved_ref(decoded);
        for frame in samples.samples().chunks_exact(count) {
            for (channel, &sample) in channels.iter_mut().zip(frame) {
                channel.push(sample);
            }
        }
    }
    if channels.is_empty() {
        return Err("EncodingError: Unable to decode audio data: no samples".into());
    }
    Ok(Decoded { rate, channels })
}
