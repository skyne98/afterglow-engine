//! Chromium command-line flag configuration for the best game-window experience.
//! Applied in `App::on_before_command_line_processing` so they propagate to all
//! (child) processes.

use cef::*;

use crate::config::CONFIG;

/// Add a bare switch if not already present.
fn sw(cl: &CommandLine, name: &str) {
    let s = CefString::from(name);
    if cl.has_switch(Some(&s)) == 0 {
        cl.append_switch(Some(&s));
    }
}
/// Add a switch with a value if not already present.
fn sv(cl: &CommandLine, name: &str, val: &str) {
    let s = CefString::from(name);
    if cl.has_switch(Some(&s)) == 0 {
        cl.append_switch_with_value(Some(&s), Some(&CefString::from(val)));
    }
}

pub fn apply(cl: &CommandLine) {
    let cfg = CONFIG.get().expect("afterglow-cef config not set");

    // --- WebGPU on the real GPU (Dawn -> Vulkan) ---
    sw(cl, "enable-unsafe-webgpu");
    sw(cl, "ignore-gpu-blocklist");
    // WebGPU is a hard engine requirement. Keep Chromium from providing a
    // WebGL backend if page-side initialization ever regresses.
    sw(cl, "disable-webgl");
    sv(cl, "enable-features", "Vulkan");
    sv(cl, "use-angle", "vulkan");

    // --- X11/XWayland: Wayland+Vulkan are incompatible in CEF 149.
    //     (Native Wayland + WebGPU isn't available yet; revisit when Chromium
    //     supports Wayland+Vulkan.) Overridable via CLI --ozone-platform=... ---
    sv(cl, "ozone-platform", "x11");

    // --- Prevent background throttling when the window is occluded (e.g.
    //     screen locked). Keeps rAF firing at full rate for headless benchmarks.
    //     Also prevents renderer process priority reduction. ---
    sw(cl, "disable-background-timer-throttling");
    sw(cl, "disable-backgrounding-occluded-windows");
    sw(cl, "disable-renderer-backgrounding");
    sw(cl, "disable-features=CalculateNativeWinOcclusion");

    // --- Latency: vsync toggle.
    //     vsync-on (default) is smooth and runs at the monitor's refresh rate
    //     (verified: 144 Hz). vsync-off was choppy on this CEF/NVIDIA/Linux
    //     setup, so it's opt-in only. ---
    if !cfg.vsync {
        sw(cl, "disable-gpu-vsync");
        sw(cl, "disable-frame-rate-limit");
    }
}
