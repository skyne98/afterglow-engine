//! Minimal CEF bootstrap, closely mirroring the upstream `cefsimple` example
//! from `tauri-apps/cef-rs`. The only behavioral additions live in
//! `simple_app.rs` (forcing WebGPU/Vulkan flags + loading a local WebGPU demo).

use cef::*;

pub mod simple_app;
pub mod simple_handler;

#[cfg(target_os = "macos")]
pub type Library = library_loader::LibraryLoader;

#[cfg(not(target_os = "macos"))]
pub struct Library;

#[allow(dead_code)]
pub fn load_cef() -> Library {
    #[cfg(target_os = "macos")]
    let library = {
        let loader = library_loader::LibraryLoader::new(&std::env::current_exe().unwrap(), false);
        assert!(loader.load());
        loader
    };
    #[cfg(not(target_os = "macos"))]
    let library = Library;

    // Initialize the CEF API version.
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    library
}

#[allow(dead_code)]
pub fn run_main(main_args: &MainArgs, cmd_line: &CommandLine, sandbox_info: *mut u8) {
    let switch = CefString::from("type");
    let is_browser_process = cmd_line.has_switch(Some(&switch)) != 1;

    let ret = execute_process(Some(main_args), None, sandbox_info);

    if is_browser_process {
        println!("[afterglow] launch browser process");
        assert_eq!(ret, -1, "cannot execute browser process");
    } else {
        let process_type = CefString::from(&cmd_line.switch_value(Some(&switch)));
        println!("[afterglow] launch process {process_type}");
        assert!(ret >= 0, "cannot execute non-browser process");
        // non-browser process does not initialize cef
        return;
    }

    let mut app = simple_app::SimpleApp::new();

    // Locate CEF resources (icudtl.dat, *.pak, locales/) from CEF_PATH so the
    // binary runs without `bundle-cef-app`. Also disable the sandbox for the
    // prototype (avoids chrome-sandbox SUID/CAP setup in dev).
    let cef_path = std::env::var("CEF_PATH").unwrap_or_default();
    let devtools_port: i32 = std::env::var("AFTERGLOW_DEVTOOLS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0); // 0 = disabled; set AFTERGLOW_DEVTOOLS=9222 to enable CDP
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
