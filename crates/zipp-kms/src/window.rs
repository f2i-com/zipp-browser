//! The one window there is: the whole screen.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use dpi::{PhysicalInsets, PhysicalPosition, PhysicalSize, Position, Size};
use raw_window_handle as rwh_06;
use winit_core::cursor::Cursor;
use winit_core::error::{NotSupportedError, RequestError};
use winit_core::icon::Icon;
use winit_core::monitor::{Fullscreen, MonitorHandle};
use winit_core::window::{
    CursorGrabMode, ImeCapabilities, ImeRequest, ImeRequestError, ResizeDirection, Theme, UserAttentionType,
    Window, WindowButtons, WindowId, WindowLevel,
};

use crate::Shared;

pub(crate) struct KmsWindow {
    pub(crate) id: WindowId,
    pub(crate) shared: Arc<Shared>,
}

impl fmt::Debug for KmsWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KmsWindow").field("id", &self.id).finish_non_exhaustive()
    }
}

fn unsupported(what: &'static str) -> RequestError {
    RequestError::NotSupported(NotSupportedError::new(what))
}

impl Window for KmsWindow {
    fn id(&self) -> WindowId {
        self.id
    }

    fn scale_factor(&self) -> f64 {
        self.shared.scale_factor
    }

    fn request_redraw(&self) {
        self.shared.redraw.store(true, Ordering::Release);
        self.shared.wake();
    }

    fn pre_present_notify(&self) {}

    fn reset_dead_keys(&self) {}

    fn surface_position(&self) -> PhysicalPosition<i32> {
        PhysicalPosition::new(0, 0)
    }

    fn outer_position(&self) -> Result<PhysicalPosition<i32>, RequestError> {
        Ok(PhysicalPosition::new(0, 0))
    }

    fn set_outer_position(&self, _position: Position) {}

    fn surface_size(&self) -> PhysicalSize<u32> {
        let (w, h) = self.shared.display.size();
        PhysicalSize::new(w, h)
    }

    fn request_surface_size(&self, _size: Size) -> Option<PhysicalSize<u32>> {
        Some(self.surface_size())
    }

    fn outer_size(&self) -> PhysicalSize<u32> {
        self.surface_size()
    }

    fn safe_area(&self) -> PhysicalInsets<u32> {
        // A widened row pitch leaves columns past the mode's right edge that
        // are drawn but never shown.
        // (Turned, the renderer fills the panel's rows itself.)
        let hidden = self.shared.display.size().0.saturating_sub(self.shared.display.visible_size().0);
        PhysicalInsets::new(0, 0, 0, hidden)
    }

    fn set_min_surface_size(&self, _min_size: Option<Size>) {}
    fn set_max_surface_size(&self, _max_size: Option<Size>) {}

    fn surface_resize_increments(&self) -> Option<PhysicalSize<u32>> {
        None
    }

    fn set_surface_resize_increments(&self, _increments: Option<Size>) {}

    fn set_title(&self, title: &str) {
        *self.shared.title.lock().unwrap() = title.to_string();
    }

    fn set_transparent(&self, _transparent: bool) {}
    fn set_blur(&self, _blur: bool) {}
    fn set_visible(&self, _visible: bool) {}

    fn is_visible(&self) -> Option<bool> {
        Some(true)
    }

    fn set_resizable(&self, _resizable: bool) {}

    fn is_resizable(&self) -> bool {
        false
    }

    fn set_enabled_buttons(&self, _buttons: WindowButtons) {}

    fn enabled_buttons(&self) -> WindowButtons {
        WindowButtons::empty()
    }

    fn set_minimized(&self, _minimized: bool) {}

    fn is_minimized(&self) -> Option<bool> {
        Some(false)
    }

    fn set_maximized(&self, _maximized: bool) {}

    fn is_maximized(&self) -> bool {
        true
    }

    fn set_fullscreen(&self, _fullscreen: Option<Fullscreen>) {}

    fn fullscreen(&self) -> Option<Fullscreen> {
        Some(Fullscreen::Borderless(None))
    }

    fn set_decorations(&self, _decorations: bool) {}

    fn is_decorated(&self) -> bool {
        false
    }

    fn set_window_level(&self, _level: WindowLevel) {}
    fn set_window_icon(&self, _window_icon: Option<Icon>) {}

    fn request_ime_update(&self, _request: ImeRequest) -> Result<(), ImeRequestError> {
        Err(ImeRequestError::NotSupported)
    }

    fn ime_capabilities(&self) -> Option<ImeCapabilities> {
        None
    }

    fn focus_window(&self) {}

    fn has_focus(&self) -> bool {
        true
    }

    fn request_user_attention(&self, _request_type: Option<UserAttentionType>) {}
    fn set_theme(&self, _theme: Option<Theme>) {}

    fn theme(&self) -> Option<Theme> {
        Some(self.shared.theme)
    }

    fn set_content_protected(&self, _protected: bool) {}

    fn title(&self) -> String {
        self.shared.title.lock().unwrap().clone()
    }

    fn set_cursor(&self, _cursor: Cursor) {}

    fn set_cursor_position(&self, _position: Position) -> Result<(), RequestError> {
        Err(unsupported("set_cursor_position"))
    }

    fn set_cursor_grab(&self, _mode: CursorGrabMode) -> Result<(), RequestError> {
        Ok(())
    }

    fn set_cursor_visible(&self, visible: bool) {
        // Shown again by the next movement.
        if !visible {
            self.shared.display.hide_pointer();
        }
    }

    fn drag_window(&self) -> Result<(), RequestError> {
        Err(unsupported("drag_window"))
    }

    fn drag_resize_window(&self, _direction: ResizeDirection) -> Result<(), RequestError> {
        Err(unsupported("drag_resize_window"))
    }

    fn show_window_menu(&self, _position: Position) {}

    fn set_cursor_hittest(&self, _hittest: bool) -> Result<(), RequestError> {
        Ok(())
    }

    fn current_monitor(&self) -> Option<MonitorHandle> {
        None
    }

    fn available_monitors(&self) -> Box<dyn Iterator<Item = MonitorHandle>> {
        Box::new(std::iter::empty())
    }

    fn primary_monitor(&self) -> Option<MonitorHandle> {
        None
    }

    fn rwh_06_display_handle(&self) -> &dyn rwh_06::HasDisplayHandle {
        &*self.shared
    }

    fn rwh_06_window_handle(&self) -> &dyn rwh_06::HasWindowHandle {
        self
    }
}

impl rwh_06::HasWindowHandle for KmsWindow {
    fn window_handle(&self) -> Result<rwh_06::WindowHandle<'_>, rwh_06::HandleError> {
        let raw = rwh_06::RawWindowHandle::Drm(rwh_06::DrmWindowHandle::new(u32::from(self.shared.display.plane)));
        // SAFETY: the plane belongs to the DRM device `Shared` keeps open for
        // as long as this window exists.
        Ok(unsafe { rwh_06::WindowHandle::borrow_raw(raw) })
    }
}
