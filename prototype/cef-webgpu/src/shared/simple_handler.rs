//! Minimal browser `Client` + `LifeSpanHandler`: tracks live browsers and
//! quits the app message loop when the last one closes. Trimmed from the
//! upstream `cefsimple` `SimpleHandler` (no load-error/display handlers).

use cef::*;
use std::sync::{Arc, Mutex, OnceLock, Weak};

static HANDLER_INSTANCE: OnceLock<Weak<Mutex<SimpleHandler>>> = OnceLock::new();

pub struct SimpleHandler {
    browser_list: Vec<Browser>,
}

impl SimpleHandler {
    pub fn new() -> Arc<Mutex<Self>> {
        Arc::new_cyclic(|weak| {
            let _ = HANDLER_INSTANCE.set(weak.clone());
            Mutex::new(Self {
                browser_list: Vec::new(),
            })
        })
    }

    fn on_after_created(&mut self, browser: Option<&mut Browser>) {
        let browser = browser.cloned().expect("Browser is None");
        self.browser_list.push(browser);
    }

    fn on_before_close(&mut self, browser: Option<&mut Browser>) {
        let mut browser = browser.cloned().expect("Browser is None");
        if let Some(index) = self
            .browser_list
            .iter()
            .position(move |elem| elem.is_same(Some(&mut browser)) != 0)
        {
            self.browser_list.remove(index);
        }
        if self.browser_list.is_empty() {
            quit_message_loop();
        }
    }
}

wrap_client! {
    pub struct SimpleHandlerClient {
        inner: Arc<Mutex<SimpleHandler>>,
    }

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(SimpleHandlerDisplayHandler::new())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(SimpleHandlerLifeSpanHandler::new(self.inner.clone()))
        }
    }
}

wrap_display_handler! {
    struct SimpleHandlerDisplayHandler;

    impl DisplayHandler {
        /// Forward JS console.* to stderr so WebGPU init status is visible
        /// in the terminal (the prototype's HUD also shows it on-screen).
        fn on_console_message(
            &self,
            _browser: Option<&mut Browser>,
            _level: LogSeverity,
            message: Option<&CefString>,
            source: Option<&CefString>,
            line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let msg = message.map(CefString::to_string).unwrap_or_default();
            let src = source.map(CefString::to_string).unwrap_or_default();
            eprintln!("[console] {msg}  ({src}:{line})");
            1
        }
    }
}

wrap_life_span_handler! {
    struct SimpleHandlerLifeSpanHandler {
        inner: Arc<Mutex<SimpleHandler>>,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let mut inner = self.inner.lock().expect("Failed to lock inner");
            inner.on_after_created(browser);
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let mut inner = self.inner.lock().expect("Failed to lock inner");
            inner.on_before_close(browser);
        }
    }
}
