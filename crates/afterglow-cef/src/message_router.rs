//! CEF Message Router (`cefQuery`): the page->host->worker bridge on native.
//!
//! Stage 1 (this file): router infrastructure + an echo handler to prove
//! `window.cefQuery` round-trips page -> host -> page. Stage 2 routes queries
//! to worker channels; Stage 3 wraps `cefQuery` in an emulated Web Worker
//! (`postMessage`/`onmessage`) so the TS client is uniform.

use cef::wrapper::message_router::{
    BrowserSideCallback, BrowserSideHandler, BrowserSideRouter, MessageRouterBrowserSide,
    MessageRouterBrowserSideHandlerCallbacks, MessageRouterConfig,
    MessageRouterRendererSide, MessageRouterRendererSideHandlerCallbacks, RendererSideRouter,
};
use cef::*;
use std::sync::{Arc, Mutex, OnceLock};

fn config() -> MessageRouterConfig {
    MessageRouterConfig::default()
}

// --- browser side ---------------------------------------------------------

static BROWSER_ROUTER: OnceLock<Arc<BrowserSideRouter>> = OnceLock::new();

pub fn browser_router() -> &'static Arc<BrowserSideRouter> {
    BROWSER_ROUTER.get_or_init(|| BrowserSideRouter::new(config()))
}

/// Create the browser-side router + register the handler. Call from
/// `on_context_initialized` (browser process UI thread).
pub fn init_browser_router() {
    let r = browser_router();
    r.add_handler(Arc::new(EchoHandler), true);
}

/// Stage-1 handler: echoes the request string back. Proves cefQuery works.
struct EchoHandler;
impl BrowserSideHandler for EchoHandler {
    fn on_query_str(
        &self,
        _browser: Option<Browser>,
        _frame: Option<Frame>,
        _query_id: i64,
        request: &str,
        _persistent: bool,
        callback: Arc<Mutex<dyn BrowserSideCallback>>,
    ) -> bool {
        eprintln!("[afterglow] cefQuery echo: {request}");
        if let Ok(cb) = callback.lock() {
            cb.success_str(request);
        }
        true
    }
}

// --- renderer side (registers window.cefQuery via V8) ---------------------
//
// cef trait methods receive `&mut` refs; the router's callbacks take owned
// values, so we clone through.

wrap_render_process_handler! {
    pub struct GameRenderProcessHandler {
        router: Arc<RendererSideRouter>,
    }

    impl RenderProcessHandler {
        fn on_context_created(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            self.router.on_context_created(
                browser.map(|b| b.clone()),
                frame.map(|f| f.clone()),
                context.map(|c| c.clone()),
            );
        }

        fn on_context_released(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            self.router.on_context_released(
                browser.map(|b| b.clone()),
                frame.map(|f| f.clone()),
                context.map(|c| c.clone()),
            );
        }

        fn on_process_message_received(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, source_process: ProcessId, message: Option<&mut ProcessMessage>) -> ::std::os::raw::c_int {
            eprintln!("[afterglow] renderer on_process_message_received");
            self.router.on_process_message_received(
                browser.map(|b| b.clone()),
                frame.map(|f| f.clone()),
                Some(source_process),
                message.map(|m| m.clone()),
            ) as i32
        }
    }
}

impl GameRenderProcessHandler {
    pub fn make() -> RenderProcessHandler {
        GameRenderProcessHandler::new(RendererSideRouter::new(config()))
    }
}

// --- browser-side lifecycle hooks (wired from the Client/RequestHandler) ---

wrap_request_handler! {
    pub struct GameRequestHandler;

    impl RequestHandler {
        fn on_before_browse(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, _request: Option<&mut Request>, _user_gesture: ::std::os::raw::c_int, _is_redirect: ::std::os::raw::c_int) -> ::std::os::raw::c_int {
            browser_router().on_before_browse(browser.map(|b| b.clone()), frame.map(|f| f.clone()));
            0
        }

        fn on_render_process_terminated(&self, browser: Option<&mut Browser>, _status: TerminationStatus, _error_code: ::std::os::raw::c_int, _error_string: Option<&CefString>) {
            browser_router().on_render_process_terminated(browser.map(|b| b.clone()));
        }
    }
}
