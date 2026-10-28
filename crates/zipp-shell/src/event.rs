//! Shell events for communication between the windowing layer and the document.
use zipp_dom::traits_src::navigation::NavigationOptions;
use zipp_dom::traits_src::net::NetWaker;
use futures_util::task::ArcWake;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::{any::Any, sync::Arc};
use winit::{event_loop::EventLoopProxy, window::WindowId};

#[cfg(feature = "accessibility")]
use accesskit_xplat::WindowEvent as AccessKitEvent;

#[derive(Debug, Clone)]
pub enum ShellEvent {
    Poll {
        window_id: WindowId,
    },

    RequestRedraw {
        doc_id: usize,
    },

    /// An accessibility event from `accesskit`.
    #[cfg(feature = "accessibility")]
    Accessibility {
        window_id: WindowId,
        data: Arc<AccessKitEvent>,
    },

    /// An arbitrary event from the embedder
    Embedder(Arc<dyn Any + Send + Sync>),

    /// Navigate to another URL (triggered by e.g. clicking a link)
    Navigate(Box<NavigationOptions>),

    /// Scroll the current document to the element a URL fragment names, without
    /// navigating: a same-document link. Reloading instead threw away the
    /// page's live JS, so nothing on it responded after the first nav click.
    ScrollToFragment { fragment: String },
    /// Navigate to another URL (triggered by e.g. clicking a link). `contents` is
    /// the page as fetched; without a document builder it is parsed as is.
    NavigationLoad {
        url: String,
        contents: String,
        retain_scroll_position: bool,
        is_md: bool,
    },
}
impl ShellEvent {
    #[inline]
    pub fn embedder_event<T: Any + Send + Sync>(value: T) -> Self {
        let boxed = Arc::new(value) as Arc<dyn Any + Send + Sync>;
        Self::Embedder(boxed)
    }
}

#[derive(Clone)]
pub struct ShellProxy(Arc<ShellProxyInner>);
pub struct ShellProxyInner {
    winit_proxy: EventLoopProxy,
    sender: Sender<ShellEvent>,
}

impl ShellProxy {
    pub fn new(winit_proxy: EventLoopProxy) -> (Self, Receiver<ShellEvent>) {
        let (sender, receiver) = channel();
        let proxy = Self(Arc::new(ShellProxyInner {
            winit_proxy,
            sender,
        }));
        (proxy, receiver)
    }

    #[inline]
    pub fn wake_up(&self) {
        self.0.winit_proxy.wake_up();
    }
    #[inline]
    pub fn send_event(&self, event: impl Into<ShellEvent>) {
        self.send_event_impl(event.into());
    }
    #[inline]
    fn send_event_impl(&self, event: ShellEvent) {
        let _ = self.0.sender.send(event);
        self.wake_up();
    }
}

impl NetWaker for ShellProxy {
    #[inline]
    fn wake(&self, client_id: usize) {
        self.send_event_impl(ShellEvent::RequestRedraw { doc_id: client_id })
    }
}

/// Create a waker that will send a poll event to the event loop.
///
/// This lets the VirtualDom "come up for air" and process events while the main thread is blocked by the WebView.
///
/// All other IO lives in the Tokio runtime,
pub fn create_waker(proxy: &ShellProxy, id: WindowId) -> std::task::Waker {
    struct DomHandle {
        proxy: ShellProxy,
        id: WindowId,
    }
    impl ArcWake for DomHandle {
        fn wake_by_ref(arc_self: &Arc<Self>) {
            let event = ShellEvent::Poll {
                window_id: arc_self.id,
            };
            arc_self.proxy.send_event(event)
        }
    }

    let proxy = proxy.clone();
    futures_util::task::waker(Arc::new(DomHandle { id, proxy }))
}
