//! CEF `App` + `BrowserProcessHandler`.
//!
//! Two customizations over upstream `cefsimple`:
//! 1. `on_before_command_line_processing` forces the WebGPU + Vulkan flags so
//!    the prototype works without manual CLI args (see
//!    docs/research/cef-wayland-vulkan-webgpu.md).
//! 2. `on_context_initialized` loads a bundled WebGPU demo (`index.html`)
//!    instead of google.com, via a `file://` URL written to a temp path.

use cef::*;
use std::cell::RefCell;

use super::simple_handler::*;

/// The WebGPU demo HTML, embedded at compile time from `resources/index.html`.
const DEMO_HTML: &str = include_str!("../../resources/index.html");

/// Write the demo HTML to a temp file and return a `file://` URL to it.
fn demo_url() -> String {
    let dir = std::env::temp_dir();
    let path = dir.join("afterglow_cef_webgpu.html");
    let _ = std::fs::write(&path, DEMO_HTML);
    format!("file://{}", path.display())
}

wrap_window_delegate! {
    struct SimpleWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 1280, height: 800 }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let browser_view = self.browser_view.borrow();
            let (Some(window), Some(browser_view)) = (window, browser_view.as_ref()) else {
                return;
            };
            let mut view = View::from(browser_view);
            window.add_child_view(Some(&mut view));
            window.show();
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            let mut browser_view = self.browser_view.borrow_mut();
            *browser_view = None;
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            let browser_view = self.browser_view.borrow();
            let browser_view = browser_view.as_ref().expect("BrowserView is None");
            if let Some(browser) = browser_view.browser() {
                let browser_host = browser.host().expect("BrowserHost is None");
                browser_host.try_close_browser()
            } else {
                1
            }
        }
    }
}

wrap_browser_view_delegate! {
    struct SimpleBrowserViewDelegate {
        runtime_style: RuntimeStyle,
    }

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn on_popup_browser_view_created(
            &self,
            _browser_view: Option<&mut BrowserView>,
            popup_browser_view: Option<&mut BrowserView>,
            _is_devtools: i32,
        ) -> i32 {
            let mut window_delegate =
                SimpleWindowDelegate::new(RefCell::new(popup_browser_view.cloned()));
            window_create_top_level(Some(&mut window_delegate));
            1
        }

        fn browser_runtime_style(&self) -> RuntimeStyle {
            self.runtime_style
        }
    }
}

wrap_app! {
    pub struct SimpleApp;

    impl App {
        /// Force WebGPU + Vulkan on for every (child) process. This is the
        /// CEF-blessed hook: switches appended here propagate to the GPU and
        /// renderer processes. User-supplied argv (e.g. `--ozone-platform=wayland`)
        /// is already present on `command_line`.
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            let Some(cl) = command_line else { return };

            // WebGPU is disabled by default in Chromium/CEF; enable it.
            let webgpu = CefString::from("enable-unsafe-webgpu");
            if cl.has_switch(Some(&webgpu)) == 0 {
                cl.append_switch(Some(&webgpu));
            }

            // Force-enable on GPUs Chromium's blocklist would skip (Steam Deck, etc.).
            let ignore = CefString::from("ignore-gpu-blocklist");
            if cl.has_switch(Some(&ignore)) == 0 {
                cl.append_switch(Some(&ignore));
            }

            // Vulkan GPU backend (Linux/Android). If the user already set
            // --enable-features, leave it; otherwise add Vulkan.
            let ef = CefString::from("enable-features");
            if cl.has_switch(Some(&ef)) == 0 {
                cl.append_switch_with_value(Some(&ef), Some(&CefString::from("Vulkan")));
            }

            // ANGLE-on-Vulkan. Let the user override via CLI if they pass --use-angle.
            let ua = CefString::from("use-angle");
            if cl.has_switch(Some(&ua)) == 0 {
                cl.append_switch_with_value(Some(&ua), Some(&CefString::from("vulkan")));
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(SimpleBrowserProcessHandler::new(RefCell::new(None)))
        }
    }
}

wrap_browser_process_handler! {
    struct SimpleBrowserProcessHandler {
        client: RefCell<Option<Client>>,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            debug_assert_ne!(currently_on(ThreadId::UI), 0);

            // SimpleHandler implements browser-level callbacks.
            {
                let mut client = self.client.borrow_mut();
                *client = Some(SimpleHandlerClient::new(SimpleHandler::new()));
            }

            // Load the bundled WebGPU demo.
            let url = CefString::from(demo_url().as_str());
            let settings = BrowserSettings::default();

            // Views framework (windowed, native Wayland-capable) is the default.
            let mut client = self.default_client();
            let mut delegate = SimpleBrowserViewDelegate::new(RuntimeStyle::DEFAULT);
            let browser_view = browser_view_create(
                client.as_mut(),
                Some(&url),
                Some(&settings),
                None,
                None,
                Some(&mut delegate),
            );

            let mut window_delegate = SimpleWindowDelegate::new(RefCell::new(browser_view));
            window_create_top_level(Some(&mut window_delegate));
        }

        fn default_client(&self) -> Option<Client> {
            self.client.borrow().clone()
        }
    }
}
