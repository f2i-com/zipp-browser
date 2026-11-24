//! `<video>` elements: starting their players and their built-in controls.
//!
//! The player itself comes from the embedder's [`MediaProvider`]; this module
//! decides when one starts, which sources it is offered, and what a click on
//! the element does.
//!
//! [`MediaProvider`]: crate::traits_src::media::MediaProvider

use std::sync::Arc;

use markup5ever::local_name;

use crate::BaseDocument;
use crate::node::MediaElement;
use crate::traits_src::media::{MediaControlsLayout, MediaOptions, MediaPlayer, MediaSource};
use crate::traits_src::shell::ShellProvider;

impl BaseDocument {
    /// The player of the `<video>` at `node_id`, if it has started.
    pub fn media_player(&self, node_id: usize) -> Option<Arc<dyn MediaPlayer>> {
        self.nodes
            .get(node_id)?
            .element_data()?
            .media
            .as_ref()
            .map(|media| media.0.clone())
    }

    /// The element's candidate resources, in the order the HTML spec tries
    /// them: its `src` if it has one, otherwise each `<source>` child.
    pub fn media_sources_for(&self, node_id: usize) -> Vec<MediaSource> {
        let Some(element) = self.nodes.get(node_id).and_then(|n| n.element_data()) else {
            return Vec::new();
        };
        if let Some(src) = element.attr(local_name!("src")).filter(|s| !s.trim().is_empty()) {
            return vec![MediaSource {
                url: self.resolve_url(src).to_string(),
                mime_type: None,
            }];
        }
        self.nodes[node_id]
            .children
            .iter()
            .filter_map(|&child| self.nodes[child].element_data())
            .filter(|child| child.name.local == local_name!("source"))
            .filter_map(|source| {
                let src = source.attr(local_name!("src")).filter(|s| !s.trim().is_empty())?;
                Some(MediaSource {
                    url: self.resolve_url(src).to_string(),
                    mime_type: source
                        .attr(local_name!("type"))
                        .map(str::to_string)
                        .filter(|t| !t.trim().is_empty()),
                })
            })
            .collect()
    }

    /// Start the element's player, or return the one it already has.
    pub fn start_media_player(&mut self, node_id: usize, autoplay: bool) -> Option<Arc<dyn MediaPlayer>> {
        if let Some(player) = self.media_player(node_id) {
            return Some(player);
        }
        let provider = self.media_provider.clone()?;
        let sources = self.media_sources_for(node_id);
        if sources.is_empty() {
            return None;
        }
        let element = self.nodes.get(node_id)?.element_data()?;
        let options = MediaOptions {
            autoplay,
            muted: element.attr(local_name!("muted")).is_some(),
            looping: element.attr(local_name!("loop")).is_some(),
        };
        let player = provider.create_player(sources, options, self.shell_provider.clone())?;
        let element = self.nodes.get_mut(node_id)?.element_data_mut()?;
        element.media = Some(MediaElement(player.clone()));
        Some(player)
    }

    /// Re-attach a preserved player (e.g. across a JS DOM re-render) to `node_id`.
    pub fn attach_media_player(&mut self, node_id: usize, player: Arc<dyn MediaPlayer>) {
        player.set_shell_provider(self.shell_provider.clone());
        if let Some(element) = self.nodes.get_mut(node_id).and_then(|n| n.element_data_mut()) {
            element.media = Some(MediaElement(player));
        }
    }

    /// Drop any active player attached to `node_id` so the next load/play recreates it.
    pub fn reset_media_player(&mut self, node_id: usize) {
        if let Some(element) = self.nodes.get_mut(node_id).and_then(|n| n.element_data_mut()) {
            element.media = None;
        }
    }

    /// A `<video>` was added to the document. A muted `autoplay` video starts
    /// at once, as in Chrome; one with `preload="metadata"` or `preload="auto"`
    /// (or default with `controls`) loads its first frame paused so it doesn't
    /// stay a blank black box until clicked.
    pub(crate) fn media_element_added(&mut self, node_id: usize) {
        let Some(element) = self.nodes.get(node_id).and_then(|n| n.element_data()) else {
            return;
        };
        let autoplay = element.attr(local_name!("autoplay")).is_some();
        let muted = element.attr(local_name!("muted")).is_some();
        if autoplay && muted {
            self.start_media_player(node_id, true);
        } else {
            let preload = element.attr(local_name!("preload")).unwrap_or("");
            if !preload.eq_ignore_ascii_case("none")
                && (element.attr(local_name!("controls")).is_some()
                    || preload.eq_ignore_ascii_case("auto")
                    || preload.eq_ignore_ascii_case("metadata")
                    || autoplay)
            {
                self.start_media_player(node_id, false);
            }
        }
    }

    /// The default action of a click on a `<video>`. With `controls`, the bar
    /// along the bottom plays/pauses, seeks and mutes; anywhere else on the
    /// video plays or pauses it.
    pub(crate) fn media_click(&mut self, node_id: usize, page_x: f32, page_y: f32) {
        let node = &self.nodes[node_id];
        let origin = node.absolute_position(0.0, 0.0);
        let size = node.final_layout.size;
        let (x, y) = (page_x - origin.x, page_y - origin.y);
        let has_controls = node
            .element_data()
            .is_some_and(|el| el.attr(local_name!("controls")).is_some());

        let Some(player) = self.start_media_player(node_id, false) else {
            return;
        };
        let status = player.status();
        let toggle = |player: &Arc<dyn MediaPlayer>| {
            if status.paused || status.ended {
                player.play();
            } else {
                player.pause();
            }
        };

        let controls = MediaControlsLayout::new(size.width, size.height);
        if has_controls && controls.bar.contains(x, y) {
            if controls.play.contains(x, y) {
                toggle(&player);
            } else if controls.mute.contains(x, y) {
                player.set_muted(!status.muted);
            } else if let Some(duration) = status.duration.filter(|d| d.is_finite() && *d > 0.0) {
                player.seek(controls.seek_fraction(x) as f64 * duration);
            }
        } else {
            toggle(&player);
        }
        self.shell_provider.request_paint();
    }

    /// Pass a new shell provider on to every playing video, so their frames
    /// reach the window this document is now shown in.
    pub(crate) fn update_media_shell_providers(&self, shell: &Arc<dyn ShellProvider>) {
        for (_, node) in self.nodes.iter() {
            if let Some(media) = node.element_data().and_then(|el| el.media.as_ref()) {
                media.0.set_shell_provider(shell.clone());
            }
        }
    }
}
