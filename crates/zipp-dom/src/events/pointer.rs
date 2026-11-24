//! Pointer (mouse/touch) event processing, hit testing, and dispatch.

use std::{
    collections::VecDeque,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::traits_src::{
    events::{
        InputEvent, PointerEvent, PointerId, WheelDelta, WheelEvent,
        DomEvent, DomEventData, MouseEventButton, MouseEventButtons,
    },
    navigation::NavigationOptions,
};
use keyboard_types::Modifiers;
use markup5ever::local_name;

use crate::{BaseDocument, node::SpecialElementData};

use super::focus::generate_focus_events;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FlingState {
    pub(crate) target: usize,
    pub(crate) last_seen_time: f64,
    pub(crate) x_velocity: f64,
    pub(crate) y_velocity: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScrollAnimationState {
    None,
    Fling(FlingState),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PanState {
    pub(crate) target: usize,
    pub(crate) last_x: f32,
    pub(crate) last_y: f32,
    pub(crate) samples: VecDeque<PanSample>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PanSample {
    pub(crate) time: u64,
    pub(crate) dx: f32,
    pub(crate) dy: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DragMode {
    /// We are not currently dragging
    None,
    /// We are currently dragging a selection (probably mouse)
    Selecting,
    /// We are currently panning the document with a drag (probably touch)
    Panning(PanState),
}

impl DragMode {
    #[inline]
    pub(crate) fn take(&mut self) -> DragMode {
        std::mem::replace(self, DragMode::None)
    }
}

impl PanState {
    fn update(&mut self, time_ms: u64, screen_x: f32, screen_y: f32) -> (f64, f64) {
        let dx = (screen_x - self.last_x) as f64;
        let dy = (screen_y - self.last_y) as f64;
        self.last_x = screen_x;
        self.last_y = screen_y;

        self.samples.push_back(PanSample {
            time: time_ms,
            // TODO: account for scroll delta not applied due to clamping
            dx: dx as f32,
            dy: dy as f32,
        });

        // Remove samples older than 100ms
        if self.samples.len() > 50 && time_ms.saturating_sub(self.samples.front().unwrap().time) > 100 {
            let idx = self
                .samples
                .partition_point(|sample| time_ms.saturating_sub(sample.time) > 100);
            // FIXME: use truncate_front once stable
            for _ in 0..idx {
                self.samples.pop_front();
            }
        }

        (dx, dy)
    }

    fn generate_fling(&self, time_ms: u64) -> Option<FlingState> {
        // Generate "fling"
        if let Some(last_sample) = self.samples.back()
            && time_ms.saturating_sub(last_sample.time) < 100
        {
            let idx = self
                .samples
                .partition_point(|sample| time_ms.saturating_sub(sample.time) > 100);

            // Compute pan_time. Will always be <= 100ms as we ignore samples older than that.
            let pan_start_time = self.samples[idx].time;
            let pan_time = time_ms.saturating_sub(pan_start_time) as f32;

            // Avoid division by 0
            if pan_time > 0.0 {
                let (pan_x, pan_y) = self
                    .samples
                    .iter()
                    .skip(idx)
                    .fold((0.0, 0.0), |(dx, dy), sample| {
                        (dx + sample.dx, dy + sample.dy)
                    });

                let x_velocity = if pan_x.abs() > pan_y.abs() {
                    pan_x / pan_time
                } else {
                    0.0
                };

                let y_velocity = if pan_y.abs() > pan_x.abs() {
                    pan_y / pan_time
                } else {
                    0.0
                };

                return Some(FlingState {
                    target: self.target,
                    last_seen_time: time_ms as f64,
                    x_velocity: x_velocity as f64 * 2.0,
                    y_velocity: y_velocity as f64 * 2.0,
                });
            }
        }

        None
    }
}

pub(crate) fn handle_pointermove<F: FnMut(DomEvent)>(
    doc: &mut BaseDocument,
    target: usize,
    event: &PointerEvent,
    mut dispatch_event: F,
) -> bool {
    let x = event.page_x();
    let y = event.page_y();
    let buttons = event.buttons;

    // `EventDriver::handle_pointer_move` already updated hover state and dispatched
    // MouseEnter/MouseLeave for this pointer position; avoid a duplicate full hit-test.
    let mut changed = false;

    // Check if we've moved enough to be considered a selection drag (2px threshold)
    if buttons != MouseEventButtons::None && doc.drag_mode == DragMode::None {
        let dx = x - doc.mousedown_position.x;
        let dy = y - doc.mousedown_position.y;
        if dx.abs() > 2.0 || dy.abs() > 2.0 {
            match event.id {
                PointerId::Mouse | PointerId::Pen => {
                    doc.drag_mode = DragMode::Selecting;
                }
                PointerId::Finger(_) => {
                    doc.drag_mode = DragMode::Panning(PanState {
                        target,
                        last_x: event.screen_x(),
                        last_y: event.screen_y(),
                        samples: VecDeque::with_capacity(200),
                    });
                }
            }
        }
    }

    if let DragMode::Panning(state) = &mut doc.drag_mode {
        let time_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let target = state.target;
        let (dx, dy) = state.update(time_ms, event.screen_x(), event.screen_y());

        let has_changed = doc.scroll_by(Some(target), dx, dy, &mut dispatch_event);
        return has_changed;
    }

    let Some(hit) = doc.hit(x, y) else {
        return changed;
    };

    if changed {
        dispatch_event(DomEvent::new(
            hit.node_id,
            DomEventData::MouseEnter(event.clone()),
        ));
    }

    if hit.node_id != target {
        return changed;
    }

    let node = &mut doc.nodes[target];
    let Some(el) = node.data.downcast_element_mut() else {
        // Handle text selection extension for non-element nodes
        if buttons != MouseEventButtons::None && doc.extend_text_selection_to_point(x, y) {
            changed = true;
        }
        return changed;
    };

    let disabled = el.attr(local_name!("disabled")).is_some();
    if disabled {
        return changed;
    }

    if let SpecialElementData::TextInput(ref mut text_input_data) = el.special_data {
        if buttons == MouseEventButtons::None {
            return changed;
        }

        let mut content_box_offset = taffy::Point {
            x: node.final_layout.padding.left + node.final_layout.border.left,
            y: node.final_layout.padding.top + node.final_layout.border.top,
        };
        if !text_input_data.is_multiline {
            let layout = text_input_data.editor.try_layout().unwrap();
            let content_box_height = node.final_layout.content_box_height();
            let input_height = layout.height() / layout.scale();
            let y_offset = ((content_box_height - input_height) / 2.0).max(0.0);

            content_box_offset.y += y_offset;
        }

        let x = (hit.x - content_box_offset.x) as f64 * doc.viewport.scale_f64();
        let y = (hit.y - content_box_offset.y) as f64 * doc.viewport.scale_f64();

        text_input_data
            .editor
            .driver(&mut doc.font_ctx.lock().unwrap(), &mut doc.layout_ctx)
            .extend_selection_to_point(x as f32, y as f32);

        changed = true;
    } else if event.is_mouse()
        && buttons != MouseEventButtons::None
        && doc.extend_text_selection_to_point(x, y)
    {
        changed = true;
    }

    changed
}

pub(crate) fn handle_pointerdown(
    doc: &mut BaseDocument,
    _target: usize,
    x: f32,
    y: f32,
    mods: Modifiers,
    dispatch_event: &mut dyn FnMut(DomEvent),
) {
    // Compute click count using the previous mousedown position (before updating)
    // This handles both double-click detection and text input word/line selection
    // TODO: For text inputs, only increment click count if click maps to the same/similar caret position
    doc.click_count = if doc
        .last_mousedown_time
        .map(|t| t.elapsed() < Duration::from_millis(500))
        .unwrap_or(false)
        && (doc.mousedown_position.x - x).abs() <= 2.0
        && (doc.mousedown_position.y - y).abs() <= 2.0
    {
        doc.click_count + 1
    } else {
        1
    };

    // Update mousedown tracking for next click and selection drag detection
    doc.last_mousedown_time = Some(Instant::now());
    doc.mousedown_position = taffy::Point { x, y };
    doc.drag_mode = DragMode::None;
    doc.scroll_animation = ScrollAnimationState::None;

    let Some(hit) = doc.hit(x, y) else {
        // Clear text selection when clicking outside any element
        doc.clear_text_selection();
        return;
    };

    // Use hit.node_id for determining the actual clicked element.
    // This may differ from `target` for anonymous blocks (which are layout children
    // but not DOM children), so we use the hit result for text selection.
    let actual_target = hit.node_id;

    // Check what kind of element we're dealing with and extract needed info
    enum ClickTarget {
        TextInput {
            content_box_offset: taffy::Point<f32>,
        },
        Disabled,
        SelectableText,
    }

    let click_target = {
        let node = &doc.nodes[actual_target];
        match node.data.downcast_element() {
            Some(el) if el.has_attr(local_name!("disabled")) => ClickTarget::Disabled,
            Some(el) => {
                if let SpecialElementData::TextInput(ref text_input_data) = el.special_data {
                    let mut content_box_offset = taffy::Point {
                        x: node.final_layout.padding.left + node.final_layout.border.left,
                        y: node.final_layout.padding.top + node.final_layout.border.top,
                    };
                    if !text_input_data.is_multiline {
                        let layout = text_input_data.editor.try_layout().unwrap();
                        let content_box_height = node.final_layout.content_box_height();
                        let input_height = layout.height() / layout.scale();
                        let y_offset = ((content_box_height - input_height) / 2.0).max(0.0);
                        content_box_offset.y += y_offset;
                    }
                    ClickTarget::TextInput { content_box_offset }
                } else {
                    ClickTarget::SelectableText
                }
            }
            None => ClickTarget::SelectableText,
        }
    };

    match click_target {
        ClickTarget::Disabled => (),
        ClickTarget::SelectableText => {
            // Handle text selection for non-input elements
            if let Some((inline_root_id, byte_offset)) = doc.find_text_position(x, y) {
                doc.set_text_selection(inline_root_id, byte_offset, inline_root_id, byte_offset);
                doc.shell_provider.request_redraw();
            } else {
                doc.clear_text_selection();
            }
        }
        ClickTarget::TextInput { content_box_offset } => {
            // Clear general text selection when focusing a text input
            doc.clear_text_selection();

            let tx = (hit.x - content_box_offset.x) as f64 * doc.viewport.scale_f64();
            let ty = (hit.y - content_box_offset.y) as f64 * doc.viewport.scale_f64();

            // Now get mutable access to the text input
            let click_count = doc.click_count;
            let node = &mut doc.nodes[actual_target];
            let el = node.data.downcast_element_mut().unwrap();
            if let SpecialElementData::TextInput(ref mut text_input_data) = el.special_data {
                let mut font_ctx = doc.font_ctx.lock().unwrap();
                let mut driver = text_input_data
                    .editor
                    .driver(&mut font_ctx, &mut doc.layout_ctx);

                match click_count {
                    1 => {
                        if mods.shift() {
                            driver.shift_click_extension(tx as f32, ty as f32);
                        } else {
                            driver.move_to_point(tx as f32, ty as f32);
                        }
                    }
                    2 => driver.select_word_at_point(tx as f32, ty as f32),
                    _ => driver.select_hard_line_at_point(tx as f32, ty as f32),
                }

                drop(font_ctx);
            }

            generate_focus_events(
                doc,
                &mut |doc| {
                    doc.set_focus_to(hit.node_id);
                },
                dispatch_event,
            );
        }
    }
}

pub(crate) fn handle_pointerup<F: FnMut(DomEvent)>(
    doc: &mut BaseDocument,
    target: usize,
    event: &PointerEvent,
    mut dispatch_event: F,
) {
    if doc.devtools().highlight_hover {
        let mut node = doc.get_node(target).unwrap();
        if event.button == MouseEventButton::Secondary {
            if let Some(parent_id) = node.layout_parent.get() {
                node = doc.get_node(parent_id).unwrap();
            }
        }
        doc.debug_log_node(node.id);
        doc.devtools_mut().highlight_hover = false;
        return;
    }

    // Reset Document's drag state to DragMode::None, storing the state
    // locally for use within this function
    let drag_mode = doc.drag_mode.take();

    // Don't dispatch click if we were doing a text selection drag or panning
    // the document with a touch. A "selection drag" only starts 2px from the
    // press, though, and a hand-held mouse drifts that far during an ordinary
    // click — which then never fired, so buttons seemed to ignore real clicks
    // while synthetic ones worked. Within a small slop it is still a click, as
    // in Chrome; a genuine drag-select travels well past it.
    const CLICK_SLOP: f32 = 6.0;
    let near_press = (event.page_x() - doc.mousedown_position.x).abs() <= CLICK_SLOP
        && (event.page_y() - doc.mousedown_position.y).abs() <= CLICK_SLOP;
    let do_click = match drag_mode {
        DragMode::None => true,
        DragMode::Selecting => near_press,
        DragMode::Panning(_) => false,
    };

    let time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    if let DragMode::Panning(state) = &drag_mode {
        if let Some(fling) = state.generate_fling(time_ms) {
            doc.scroll_animation = ScrollAnimationState::Fling(fling);
            doc.shell_provider.request_redraw();
        }
    }

    // Dispatch a click event
    if do_click && event.button == MouseEventButton::Main {
        dispatch_event(DomEvent::new(target, DomEventData::Click(event.clone())));
    }

    // Middle-click on an anchor opens the link in a new tab (or closes a tab
    // when the embedder's tab strip handles it).
    if do_click && event.button == MouseEventButton::Auxiliary {
        let mut maybe_node_id = Some(target);
        while let Some(node_id) = maybe_node_id {
            if let Some(el) = doc.nodes[node_id].data.downcast_element()
                && el.name.local == local_name!("a")
                && let Some(href) = el.attr(local_name!("href"))
            {
                if let Some(url) = doc.url.resolve_relative(href) {
                    let opts = NavigationOptions::new(url, "text/plain".to_owned(), doc.id())
                        .set_new_tab(true)
                        .set_background(!event.mods.contains(Modifiers::SHIFT));
                    doc.navigation_provider.navigate_to(opts);
                }
                break;
            }
            maybe_node_id = doc.nodes[node_id].parent;
        }
    }

    // Dispatch a context menu event
    if do_click && event.button == MouseEventButton::Secondary {
        dispatch_event(DomEvent::new(
            target,
            DomEventData::ContextMenu(event.clone()),
        ));
    }
}

pub(crate) fn handle_click(
    doc: &mut BaseDocument,
    target: usize,
    event: &PointerEvent,
    dispatch_event: &mut dyn FnMut(DomEvent),
) {
    let double_click_event = event.clone();
    // The page cancelled the pointer down: the click leaves focus alone.
    let keep_focus = doc.pointerdown_prevented;

    let mut maybe_node_id = Some(target);
    // Whether the click gave focus to something on the way up.
    let mut focused = false;
    let matched = 'matched: {
        while let Some(node_id) = maybe_node_id {
            let maybe_element = {
                let node = &mut doc.nodes[node_id];
                node.data.downcast_element_mut()
            };

            let Some(el) = maybe_element else {
                maybe_node_id = doc.nodes[node_id].parent;
                continue;
            };

            let disabled = el.attr(local_name!("disabled")).is_some();
            if disabled {
                break 'matched true;
            }

            if let SpecialElementData::TextInput(_) = el.special_data {
                break 'matched true;
            }

            match el.name.local {
                local_name!("input") if el.attr(local_name!("type")) == Some("checkbox") => {
                    let is_checked = BaseDocument::toggle_checkbox(el);
                    let value = if is_checked { "true".to_owned() } else { "false".to_owned() };
                    dispatch_event(DomEvent::new(
                        node_id,
                        DomEventData::Input(InputEvent { value }),
                    ));
                    generate_focus_events(
                        doc,
                        &mut |doc| {
                            doc.set_focus_to(node_id);
                        },
                        dispatch_event,
                    );
                    break 'matched true;
                }
                local_name!("input") if el.attr(local_name!("type")) == Some("radio") => {
                    if let Some(radio_set) = el.attr(local_name!("name")).map(str::to_string) {
                        BaseDocument::toggle_radio(doc, radio_set, node_id);
                    } else if let Some(checked) = doc
                        .get_node_mut(node_id)
                        .and_then(|n| n.element_data_mut())
                        .and_then(|e| e.checkbox_input_checked_mut())
                    {
                        *checked = true;
                    }

                    // TODO: make input event conditional on value actually changing
                    let value = "true".to_owned();
                    dispatch_event(DomEvent::new(
                        node_id,
                        DomEventData::Input(InputEvent { value }),
                    ));

                    generate_focus_events(
                        doc,
                        &mut |doc| {
                            doc.set_focus_to(node_id);
                        },
                        dispatch_event,
                    );

                    break 'matched true;
                }
                // Clicking labels triggers click, and possibly input event, of associated input
                local_name!("label") => {
                    if let Some(target_node_id) =
                        doc.label_bound_input_element(node_id).map(|n| n.id)
                    {
                        // Apply default click event action for target node
                        let target_node = doc.get_node_mut(target_node_id).unwrap();
                        let syn_event = target_node.synthetic_click_event_data(event.mods);
                        handle_click(doc, target_node_id, &syn_event, dispatch_event);
                        break 'matched true;
                    }
                }
                local_name!("video") => {
                    doc.media_click(node_id, event.page_x(), event.page_y());
                    break 'matched true;
                }
                local_name!("a") => {
                    if let Some(href) = el.attr(local_name!("href")) {
                        let modifier_new_tab = event.mods.contains(Modifiers::CONTROL)
                            || event.mods.contains(Modifiers::META);
                        let target_blank = el
                            .attr(local_name!("target"))
                            .is_some_and(|t| t.eq_ignore_ascii_case("_blank"));
                        if let Some(url) = doc.url.resolve_relative(href) {
                            let mut opts = NavigationOptions::new(
                                url,
                                "text/plain".to_owned(),
                                doc.id(),
                            );
                            if modifier_new_tab {
                                opts = opts
                                    .set_new_tab(true)
                                    .set_background(!event.mods.contains(Modifiers::SHIFT));
                            } else if target_blank {
                                opts = opts.set_new_tab(true).set_background(false);
                            }
                            doc.navigation_provider.navigate_to(opts);
                        } else {
                            #[cfg(feature = "tracing")]
                            tracing::warn!("{href} is not parseable as a url. : {:?}", *doc.url);
                        }
                        break 'matched true;
                    } else {
                        #[cfg(feature = "tracing")]
                        tracing::info!("Clicked link without href: {:?}", el.attrs());
                    }
                }
                local_name!("button") | local_name!("input")
                    if el.is_submit_button()
                        || (el.name.local == local_name!("input")
                            && el.attr(local_name!("type")) == Some("submit")) =>
                {
                    if let Some(form_owner) = doc.controls_to_form.get(&node_id) {
                        doc.submit_form(*form_owner, node_id);
                    }
                    break 'matched true;
                }
                #[cfg(feature = "file_input")]
                local_name!("input") if el.attr(local_name!("type")) == Some("file") => {
                    use crate::qual_name;
                    let multiple = el.attr(local_name!("multiple")).is_some();
                    // `webkitdirectory`: a folder, and every file in it.
                    let folder = el.attr(markup5ever::LocalName::from("webkitdirectory")).is_some();
                    let filter = el.attr(local_name!("accept")).and_then(accept_filter);
                    // Each chosen file, and its path from the chosen folder
                    // (`webkitRelativePath`; empty for a file chosen alone).
                    let chosen: Vec<(std::path::PathBuf, String)> = if folder {
                        doc.shell_provider.open_folder_dialog().map(folder_files).unwrap_or_default()
                    } else {
                        doc.shell_provider
                            .open_file_dialog(multiple, filter)
                            .into_iter()
                            .map(|p| (p, String::new()))
                            .collect()
                    };
                    let files: Vec<std::path::PathBuf> = chosen.iter().map(|(p, _)| p.clone()).collect();

                    let el = doc.nodes[node_id].data.downcast_element_mut().unwrap();
                    if let Some(file) = files.first() {
                        el.attrs
                            .set(qual_name!("value", html), &file.to_string_lossy());
                    }
                    let text_content = match files.len() {
                        0 => "No Files Selected".to_string(),
                        1 => files
                            .first()
                            .unwrap()
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                        x => format!("{x} Files Selected"),
                    };

                    if files.is_empty() {
                        el.special_data = SpecialElementData::None;
                    } else {
                        el.special_data = SpecialElementData::FileInput(files.into())
                    }
                    // The control's own label, where it has one (a hidden
                    // input, as pages put inside a styled <label>, may not).
                    let label_text = doc.nodes[node_id]
                        .children
                        .get(1)
                        .and_then(|&label| doc.nodes[label].children.first().copied());
                    if let Some(text) = label_text.and_then(|id| doc.nodes[id].text_data_mut()) {
                        text.content = text_content;
                    }
                    // The page hears what was chosen: an input event carrying
                    // the files as JSON `[[path, relativePath], ...]`, which
                    // its event bridge turns into `files` and `change`.
                    if !chosen.is_empty() {
                        let mut json = String::from("[");
                        for (i, (path, relative)) in chosen.iter().enumerate() {
                            if i > 0 {
                                json.push(',');
                            }
                            json.push('[');
                            json_string(&mut json, &path.to_string_lossy());
                            json.push(',');
                            json_string(&mut json, relative);
                            json.push(']');
                        }
                        json.push(']');
                        dispatch_event(DomEvent::new(node_id, DomEventData::Input(InputEvent { value: json })));
                    }
                    // Done: walking on up would reach the <label> the input
                    // sits in, whose click opens the dialog again -- over and
                    // over, Cancel or not.
                    break 'matched true;
                }
                // Anything else that takes focus -- a button, an element with
                // a `tabindex` (a canvas a game reads keys on) -- gets it, as a
                // click gives it in any browser. Before, only text fields did.
                // The walk goes on: a button inside a link still follows it.
                _ if el.is_focussable && !focused && !keep_focus => {
                    generate_focus_events(
                        doc,
                        &mut |doc| {
                            doc.set_focus_to(node_id);
                        },
                        dispatch_event,
                    );
                    focused = true;
                }
                _ => {}
            }

            // No match. Recurse up to parent.
            maybe_node_id = doc.nodes[node_id].parent;
        }

        // Didn't match anything
        false
    };

    // If nothing is matched (or focused) then clear focus
    if !matched && !focused && !keep_focus {
        generate_focus_events(doc, &mut |doc| doc.clear_focus(), dispatch_event);
    }

    // Dispatch double-click event if this is the second click in quick succession
    // (click_count was already computed in handle_mousedown)
    if doc.click_count == 2 {
        dispatch_event(DomEvent::new(
            target,
            DomEventData::DoubleClick(double_click_event),
        ));
    }
}

/// A file dialog filter from an `accept` attribute's extensions (`.py,.js`);
/// none when it names only types (`image/*`).
#[cfg(feature = "file_input")]
fn accept_filter(accept: &str) -> Option<crate::traits_src::shell::FileDialogFilter> {
    let extensions: Vec<String> = accept
        .split(',')
        .filter_map(|a| a.trim().strip_prefix('.').map(|e| e.to_ascii_lowercase()))
        .filter(|e| !e.is_empty())
        .collect();
    if extensions.is_empty() {
        return None;
    }
    Some(crate::traits_src::shell::FileDialogFilter { name: "Accepted files".into(), extensions })
}

/// Every file under `root`, with its path from `root`'s parent
/// (`folder/sub/file.py`), as `webkitRelativePath` has it. At most 10,000.
#[cfg(feature = "file_input")]
fn folder_files(root: std::path::PathBuf) -> Vec<(std::path::PathBuf, String)> {
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = Vec::new();
    let mut stack = vec![(root, name)];
    while let Some((dir, relative)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let child = format!("{relative}/{}", entry.file_name().to_string_lossy());
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push((path, child)),
                Ok(t) if t.is_file() => {
                    out.push((path, child));
                    if out.len() >= 10_000 {
                        return out;
                    }
                }
                _ => {}
            }
        }
    }
    out
}

#[cfg(feature = "file_input")]
fn json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

pub(crate) fn handle_wheel<F: FnMut(DomEvent)>(
    doc: &mut BaseDocument,
    _: usize,
    event: WheelEvent,
    mut dispatch_event: F,
) {
    let (scroll_x, scroll_y) = match event.delta {
        WheelDelta::Lines(x, y) => (x * 20.0, y * 20.0),
        WheelDelta::Pixels(x, y) => (x, y),
    };

    let has_changed = doc.scroll_by(
        doc.get_hover_node_id(),
        scroll_x,
        scroll_y,
        &mut dispatch_event,
    );
    if has_changed {
        doc.shell_provider.request_redraw();
    }
}

#[cfg(all(test, feature = "file_input"))]
mod tests {
    use super::{accept_filter, folder_files};

    #[test]
    fn a_picked_folder_lists_every_file_with_its_path_from_the_folder() {
        let root = std::env::temp_dir().join(format!("zipp-folder-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let proj = root.join("proj");
        std::fs::create_dir_all(proj.join("sub")).unwrap();
        std::fs::write(proj.join("main.py"), "print(1)").unwrap();
        std::fs::write(proj.join("sub").join("util.py"), "x = 1").unwrap();

        let mut relative: Vec<String> = folder_files(proj.clone()).into_iter().map(|(_, r)| r).collect();
        relative.sort();
        assert_eq!(relative, ["proj/main.py", "proj/sub/util.py"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn accept_gives_a_filter_of_its_extensions_only() {
        let filter = accept_filter(".py, .JS,.mjs").unwrap();
        assert_eq!(filter.extensions, ["py", "js", "mjs"]);
        assert!(accept_filter("image/*").is_none());
    }
}
