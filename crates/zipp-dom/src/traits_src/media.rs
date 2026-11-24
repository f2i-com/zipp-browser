//! Media playback for `<video>`.
//!
//! Decoding is the embedder's job: the document asks a [`MediaProvider`] for a
//! [`MediaPlayer`] when a video starts, and paints the frames it produces. A
//! document with no provider (headless screenshots, tests, other platforms)
//! simply shows the poster.

use std::sync::Arc;

use crate::node::RasterImageData;
use crate::traits_src::shell::ShellProvider;

/// One candidate resource for a media element: its `src`, or a `<source>`.
#[derive(Clone, Debug)]
pub struct MediaSource {
    /// Absolute URL.
    pub url: String,
    /// The `type` attribute, e.g. `video/webm; codecs="vp9, opus"`.
    pub mime_type: Option<String>,
}

/// How a player starts out.
#[derive(Clone, Copy, Debug, Default)]
pub struct MediaOptions {
    /// Start playing as soon as possible.
    pub autoplay: bool,
    /// Start muted.
    pub muted: bool,
    /// Restart from the beginning at the end.
    pub looping: bool,
}

/// A snapshot of a player's state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaStatus {
    pub paused: bool,
    pub ended: bool,
    pub muted: bool,
    /// Volume in `0.0..=1.0`.
    pub volume: f64,
    /// Playback speed multiplier (`1.0` normal).
    pub playback_rate: f64,
    /// HTMLMediaElement `readyState` (`0` = HAVE_NOTHING .. `4` = HAVE_ENOUGH_DATA).
    pub ready_state: u8,
    /// Seconds from the start.
    pub position: f64,
    /// Seconds, once known; `None` before metadata or for a live stream.
    pub duration: Option<f64>,
    /// The video's natural size, once known.
    pub video_size: Option<(u32, u32)>,
    /// No source could be played.
    pub error: bool,
}

impl Default for MediaStatus {
    fn default() -> Self {
        Self {
            paused: true,
            ended: false,
            muted: false,
            volume: 1.0,
            playback_rate: 1.0,
            ready_state: 0,
            position: 0.0,
            duration: None,
            video_size: None,
            error: false,
        }
    }
}

/// A playing (or paused) media resource, owned by its element. Dropping the
/// last handle stops playback and frees the decoder.
pub trait MediaPlayer: Send + Sync {
    fn play(&self);
    fn pause(&self);
    fn seek(&self, seconds: f64);
    fn set_muted(&self, muted: bool);
    fn set_volume(&self, volume: f64) {
        let _ = volume;
    }
    fn set_playback_rate(&self, rate: f64) {
        let _ = rate;
    }
    fn set_loop(&self, looping: bool) {
        let _ = looping;
    }
    fn status(&self) -> MediaStatus;
    /// The latest decoded frame, if any. Cheap: the pixels are shared.
    fn frame(&self) -> Option<RasterImageData>;
    /// The size, in device pixels, the video is painted at. Frames need be no
    /// bigger: a player may scale them down to it before handing them over.
    fn set_display_size(&self, width: u32, height: u32) {
        let _ = (width, height);
    }
    /// Where to ask for a repaint when a frame arrives. The document passes its
    /// window's provider on as soon as it has one.
    fn set_shell_provider(&self, shell: Arc<dyn ShellProvider>);
}

/// Creates players. Supplied by the embedder through `DocumentConfig`.
pub trait MediaProvider: Send + Sync + 'static {
    /// A player for the first of `sources` that can be played, or `None` if
    /// playback is unavailable altogether.
    fn create_player(
        &self,
        sources: Vec<MediaSource>,
        options: MediaOptions,
        shell: Arc<dyn ShellProvider>,
    ) -> Option<Arc<dyn MediaPlayer>>;
}

/// A rectangle in CSS pixels, relative to the video's border box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ControlRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ControlRect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Where the built-in controls of a `<video controls>` sit. Shared by the
/// painter and the click handling, so what is drawn is what is hit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MediaControlsLayout {
    /// The bar along the bottom.
    pub bar: ControlRect,
    /// Play/pause, at the bar's left end.
    pub play: ControlRect,
    /// Mute, at the bar's right end.
    pub mute: ControlRect,
    /// The progress track between them. The whole bar between the two buttons
    /// seeks; this is just the line that is drawn.
    pub track: ControlRect,
}

impl MediaControlsLayout {
    pub fn new(width: f32, height: f32) -> Self {
        let bar_height = 32.0_f32.min(height);
        let top = height - bar_height;
        let bar = ControlRect { x: 0.0, y: top, width, height: bar_height };
        let play = ControlRect { x: 0.0, y: top, width: bar_height, height: bar_height };
        let mute = ControlRect {
            x: (width - bar_height).max(0.0),
            y: top,
            width: bar_height,
            height: bar_height,
        };
        let track = ControlRect {
            x: bar_height + 4.0,
            y: top + bar_height / 2.0 - 2.0,
            width: (width - 2.0 * bar_height - 8.0).max(0.0),
            height: 4.0,
        };
        Self { bar, play, mute, track }
    }

    /// The position a click at `x` on the bar seeks to, as a fraction.
    pub fn seek_fraction(&self, x: f32) -> f32 {
        if self.track.width <= 0.0 {
            return 0.0;
        }
        ((x - self.track.x) / self.track.width).clamp(0.0, 1.0)
    }
}
