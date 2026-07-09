//! CEF bootstrap + delegates. Internal — driven by [`crate::config::CONFIG`].

use cef::*;
use std::cell::RefCell;
use std::sync::Mutex;

use crate::config::CONFIG;
use crate::flags;
use crate::input::{InputEvent, InputKind};
use cef::sys::XEvent;

/// Global handle to the main browser, set in `on_after_created`.
/// Other threads (e.g., the game loop) can use this to access the main frame
/// for `push_frame_data` etc.
pub static MAIN_BROWSER: Mutex<Option<Browser>> = Mutex::new(None);

/// Entry: set config, register the scheme, init CEF, run the message loop.
pub fn run(cfg: crate::config::Config) {
    if CONFIG.set(cfg).is_err() {
        panic!("afterglow-cef::run called twice");
    }

    let _library = load_cef();
    let args = cef::args::Args::new();
    let Some(cmd_line) = args.as_cmd_line() else {
        panic!("afterglow-cef: failed to parse command-line arguments");
    };
    run_main(args.as_main_args(), &cmd_line, std::ptr::null_mut());
}

#[cfg(target_os = "macos")]
pub type Library = library_loader::LibraryLoader;
#[cfg(not(target_os = "macos"))]
pub struct Library;

fn load_cef() -> Library {
    #[cfg(target_os = "macos")]
    let library = {
        let loader = library_loader::LibraryLoader::new(&std::env::current_exe().unwrap(), false);
        assert!(loader.load());
        loader
    };
    #[cfg(not(target_os = "macos"))]
    let library = Library;

    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    library
}

fn run_main(main_args: &MainArgs, cmd_line: &CommandLine, sandbox_info: *mut u8) {
    let switch = CefString::from("type");
    let is_browser_process = cmd_line.has_switch(Some(&switch)) != 1;

    let mut app = GameApp::new();
    let ret = execute_process(Some(main_args), Some(&mut app), sandbox_info);

    if is_browser_process {
        assert_eq!(ret, -1, "cannot execute browser process");
    } else {
        assert!(ret >= 0, "cannot execute non-browser process");
        return;
    }

    let cef_path = std::env::var("CEF_PATH").unwrap_or_default();
    let devtools_port = CONFIG.get().unwrap().devtools_port;
    let settings = Settings {
        no_sandbox: 1,
        remote_debugging_port: devtools_port,
        resources_dir_path: CefString::from(cef_path.as_str()),
        locales_dir_path: CefString::from(format!("{cef_path}/locales").as_str()),
        ..Default::default()
    };
    assert_eq!(
        initialize(Some(main_args), Some(&settings), Some(&mut app), sandbox_info),
        1
    );

    run_message_loop();
    shutdown();
}

// --- window delegate (Views framework = windowed, native Wayland-capable) --

wrap_window_delegate! {
    struct GameWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            let cfg = CONFIG.get().unwrap();
            Size { width: cfg.width, height: cfg.height }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let bv = self.browser_view.borrow();
            let (Some(window), Some(bv)) = (window, bv.as_ref()) else { return };
            let mut view = View::from(bv);
            window.add_child_view(Some(&mut view));
            window.show();
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            *self.browser_view.borrow_mut() = None;
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            let bv = self.browser_view.borrow();
            let Some(bv) = bv.as_ref() else { return 1 };
            bv.browser().and_then(|b| b.host()).map(|h| h.try_close_browser()).unwrap_or(1)
        }
    }
}

wrap_browser_view_delegate! {
    struct GameBrowserViewDelegate {
        runtime_style: RuntimeStyle,
    }

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn on_popup_browser_view_created(
            &self,
            _bv: Option<&mut BrowserView>,
            popup: Option<&mut BrowserView>,
            _is_devtools: i32,
        ) -> i32 {
            let mut d = GameWindowDelegate::new(RefCell::new(popup.cloned()));
            window_create_top_level(Some(&mut d));
            1
        }

        fn browser_runtime_style(&self) -> RuntimeStyle { self.runtime_style }
    }
}

// --- app: flags, custom scheme, browser process handler --------------------

wrap_app! {
    pub struct GameApp;

    impl App {
        fn on_before_command_line_processing(&self, _pt: Option<&CefString>, cl: Option<&mut CommandLine>) {
            if let Some(cl) = cl { flags::apply(cl); }
        }

        /// Register `afterglow://` as standard + secure + CORS + fetch so ES
        /// modules and WebGPU work on it.
        fn on_register_custom_schemes(&self, registrar: Option<&mut SchemeRegistrar>) {
            if let Some(r) = registrar {
                let name = CefString::from(crate::config::SCHEME);
                // STANDARD(1) | SECURE(8) | CORS_ENABLED(16) | CSP_BYPASSING(32) | FETCH_ENABLED(64) = 121
                r.add_custom_scheme(Some(&name), 121);
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(GameBrowserProcessHandler::new(RefCell::new(None)))
        }

        /// Renderer side: receives the shared memory region and exposes it
        /// as `window.__afterglow_buffer` (a V8 ArrayBuffer backed by the
        /// shared memory — zero-copy).
        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(crate::shared_memory::GameRenderProcessHandler::make())
        }
    }
}

wrap_browser_process_handler! {
    struct GameBrowserProcessHandler {
        client: RefCell<Option<Client>>,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            debug_assert_ne!(currently_on(ThreadId::UI), 0);

            // Serve assets directly via the afterglow:// scheme.
            crate::resources::register_factory();

            {
                let mut client = self.client.borrow_mut();
                *client = Some(GameClient::new());
            }

            let cfg = CONFIG.get().unwrap();
            let url = CefString::from(crate::root_url(&cfg.root_path).as_str());
            let settings = BrowserSettings::default();

            let mut client = self.default_client();
            let mut delegate = GameBrowserViewDelegate::new(RuntimeStyle::DEFAULT);
            let browser_view = browser_view_create(
                client.as_mut(),
                Some(&url),
                Some(&settings),
                None,
                None,
                Some(&mut delegate),
            );

            let mut window_delegate = GameWindowDelegate::new(RefCell::new(browser_view));
            window_create_top_level(Some(&mut window_delegate));
        }

        fn default_client(&self) -> Option<Client> {
            self.client.borrow().clone()
        }
    }
}

// --- client + life-span + console-forwarding display handler + keyboard ----

wrap_client! {
    pub struct GameClient;

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(GameDisplayHandler::new())
        }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(GameLifeSpanHandler::new(RefCell::new(Vec::new())))
        }
        /// Capture keyboard input natively (before the page) -> the game loop's
        /// input channel. No web/JS messages.
        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(GameKeyboardHandler::new())
        }
    }
}

wrap_life_span_handler! {
    struct GameLifeSpanHandler {
        browsers: RefCell<Vec<Browser>>,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let mut b = browser.cloned().expect("Browser is None");
            self.browsers.borrow_mut().push(b.clone());

            // Expose the main browser globally so other threads (game loop,
            // push thread) can access the main frame.
            *MAIN_BROWSER.lock().unwrap() = Some(b.clone());

            // Send the shared memory ring buffer to the renderer now that
            // the browser + main frame exist.
            if let Some(frame) = b.main_frame() {
                crate::shared_memory::send_shared_buffer(&frame, 8 * 1024 * 1024);
            }

            // Fire the user's ready callback (spawn game-loop / push threads).
            if let Some(cb) = &crate::config::CONFIG.get().unwrap().ready {
                cb();
            }
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let mut b = browser.cloned().expect("Browser is None");
            let mut list = self.browsers.borrow_mut();
            if let Some(i) = list.iter().position(|e| e.is_same(Some(&mut b)) != 0) {
                list.remove(i);
            }
            if list.is_empty() {
                quit_message_loop();
            }
        }
    }
}

wrap_display_handler! {
    struct GameDisplayHandler;

    impl DisplayHandler {
        fn on_console_message(
            &self,
            _browser: Option<&mut Browser>,
            _level: LogSeverity,
            message: Option<&CefString>,
            _source: Option<&CefString>,
            _line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let msg = message.map(CefString::to_string).unwrap_or_default();
            if let Some(cb) = CONFIG.get().and_then(|c| c.console.as_ref()) {
                cb(&msg);
            } else {
                eprintln!("[console] {msg}");
            }
            1
        }
    }
}

wrap_keyboard_handler! {
    struct GameKeyboardHandler;

    impl KeyboardHandler {
        fn on_pre_key_event(&self, _browser: Option<&mut Browser>, event: Option<&KeyEvent>, _os_event: Option<&mut XEvent>, _is_keyboard_shortcut: Option<&mut ::std::os::raw::c_int>) -> ::std::os::raw::c_int {
            if let Some(e) = event {
                let kind = if e.type_ == KeyEventType::KEYUP { InputKind::KeyUp }
                    else if e.type_ == KeyEventType::CHAR { InputKind::Char }
                    else { InputKind::KeyDown };
                let ev = InputEvent { kind, key_code: e.windows_key_code, modifiers: e.modifiers };
                crate::input::push_input(ev);
            }
            0
        }
    }
}
