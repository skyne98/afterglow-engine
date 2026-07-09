# Steam Overlay with CEF — How It Works and How to Make It Work

> Investigated 2026-07

## How the Steam Overlay works

The Steam Overlay renders on top of the game's frame by hooking the graphics
API's "present" function — the call that submits a finished frame to the
display:

| Platform | Graphics API | Hook target | Injection mechanism |
|----------|-------------|-------------|-------------------|
| Windows | D3D 9-12 | `IDirect3DDevice::Present` / `IDXGISwapChain::Present` | `GameOverlayRenderer.dll` injected into game process |
| Windows | OpenGL | `wglSwapBuffers` | Same DLL |
| Windows | Vulkan | `vkQueuePresentKHR` | Same DLL |
| Linux | Vulkan | `vkQueuePresentKHR` | `steamoverlayvulkanlayer.so` (Vulkan layer) |
| Linux | OpenGL | `glXSwapBuffers` | `gameoverlayrenderer.so` |

The overlay is loaded when the game calls `SteamAPI_Init()`. **This must
happen BEFORE the graphics device is created**, or the overlay can't hook
device creation. (Source: [Steamworks docs](https://partner.steamgames.com/doc/features/overlay))

The overlay draws its UI (friends list, chat, browser, achievements) on top
of the game's frame, then lets the present call proceed. Input is intercepted
while the overlay is open.

## Why it doesn't work with CEF/Electron/WebView2

The fundamental problem: **browsers use multi-process architecture**. A
separate **GPU process** handles rendering. The Steam overlay is injected
into the **main process** (where `SteamAPI_Init` is called). The GPU process
is a different OS process — the overlay can't hook its rendering calls.

This affects:
- CEF (our case)
- Electron
- NW.js
- WebView2
- WKWebView (macOS)

Valve acknowledges this in their docs:
> "The fundamental problem appears to be that browsers use a multi-process
> architecture. They use a separate GPU process for rendering, so no drawing
> is actually done in the main process."

## Workarounds

### Option A: `--in-process-gpu` (simplest, recommended first try)

Forces Chromium to run the GPU in the **main process** (no separate GPU
process). Steam's overlay can then hook `vkQueuePresentKHR` (Linux) or
`Present` (Windows) because rendering happens in the same process where
`SteamAPI_Init` was called.

```rust
// In afterglow-cef/src/flags.rs:
sw(cl, "in-process-gpu");
```

```rust
// In main(), before AppBuilder::run():
steam_api::init(); // calls SteamAPI_Init()
```

**Pros:**
- One flag, minimal code
- Works on Windows and Linux
- This is what Electron apps use (confirmed working by multiple developers)
- Construct game engine reports overlay works on Steam Deck with CEF

**Cons:**
- GPU crashes crash the whole app (no process isolation)
- May have stability issues with heavy GPU workloads
- Not recommended by Chromium for production, but acceptable for games

### Option B: OSR + native render window (Valve's recommended approach)

Use CEF's Offscreen Rendering (OSR) mode. CEF renders web content to a
bitmap/texture. A native D3D/Vulkan/OpenGL window draws that texture each
frame. Steam's overlay hooks the native window's present call.

Valve's docs:
> "A workaround for web based games is to host an embedded Chromium inside a
> native application, with a D3D window and input forwarding to the embedded
> Chromium. That can be setup to render in offscreen mode, which then renders
> the resulting chromium texture each frame in the native app."

**Pros:**
- Full overlay support (Valve's official recommendation)
- Full process isolation (GPU process stays separate)

**Cons:**
- Complex to implement (OSR texture management, input forwarding)
- Adds a texture copy per frame (we previously rejected OSR for this reason)
  - But: cef-rs has `accelerated_osr` zero-copy path (see
    `docs/research/cef-games-latency-footprint-debugging.md`)

### Option C: Current windowed CEF (may not work)

Our current approach: CEF creates a native window, the GPU process renders
directly to it. On Linux with X11 + Vulkan, the GPU process creates a Vulkan
swapchain.

**Does the overlay hook the GPU process?** Evidence says **no** — the overlay
is in the main process, the GPU process is separate. But some reports suggest
it works on Steam Deck (possibly because Steam Deck uses a special Proton/
Vulkan layer setup).

**Verdict:** Don't rely on this. Use `--in-process-gpu` or OSR.

## Recommendation for afterglow-engine

1. **Try `--in-process-gpu` first.** Add the flag to `flags.rs`. Call
   `SteamAPI_Init()` in `main()` before `AppBuilder::run()`. Test on Windows
   and Linux. This is the minimal-effort approach and works for Electron apps.

2. **If `--in-process-gpu` is unstable**, implement OSR with cef-rs's
   `accelerated_osr` zero-copy path. This is Valve's recommended approach but
   requires more work (texture management, input forwarding, native render
   window).

3. **SteamAPI_Init must be called before CEF init.** The overlay hooks
   graphics device creation. If `SteamAPI_Init` is called after CEF
   initializes the GPU process, the overlay won't hook it.

### Implementation sketch (Option A)

```rust
// In the game's main():
fn main() {
    // 1. Initialize Steam BEFORE CEF (overlay hooks device creation)
    steam_api::init(); // SteamAPI_Init()

    // 2. Run the CEF app (with --in-process-gpu flag)
    afterglow_cef::AppBuilder::new()
        .title("My Game")
        .root("/index.html")
        .asset("/index.html", "text/html", HTML)
        .run();
}

// In afterglow-cef/src/flags.rs:
pub fn apply(cl: &CommandLine) {
    // ... existing flags ...
    sw(cl, "in-process-gpu"); // Steam Overlay: GPU in main process
}
```

### Steam integration (steamworks crate)

Use the [`steamworks`](https://crates.io/crates/steamworks) crate (Rust
bindings for Steamworks SDK) or link `steam_api.so` / `steam_api64.lib`
directly.

Key calls:
- `SteamAPI_Init()` — initializes Steam + loads overlay renderer
- `SteamAPI_RunCallbacks()` — call each frame (in the game loop)
- `SteamAPI_Shutdown()` — on exit

The overlay appears automatically once `SteamAPI_Init` is called and the
game is launched through Steam (with a valid `steam_appid.txt`).

## Sources

- [Steamworks Overlay docs](https://partner.steamgames.com/doc/features/overlay)
- [Steam community: browser-based games not supported](https://steamcommunity.com/discussions/forum/10/591756872987476379/)
- [Jake Andreoli: Enabling Steam Overlay in Electron](https://jake.software/enabling-the-steam-overlay-in-an-electron-app)
- [JamesMoulang: Electron + Steam notes](https://github.com/JamesMoulang/electron-steam-notes)
- [Construct: Using CEF for Steam Overlay](https://www.construct.net/en/forum/construct-3/general-discussion-7/using-cef-windows-too-fix-185095)
- [ValveSoftware/steam-for-linux: overlay Vulkan layer issues](https://github.com/ValveSoftware/steam-for-linux/issues/8020)
- [aixxe: Rendering with the Steam overlay](https://aixxe.net/2017/09/steam-overlay-rendering)
- [Reddit: How does the Steam Overlay Layer know when to draw?](https://www.reddit.com/r/vulkan/comments/4v3amz/)
