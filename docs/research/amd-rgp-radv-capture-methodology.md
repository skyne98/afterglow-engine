# AMD RGP capture methodology for CEF/WebGPU on RADV

Date validated: 2026-07-21

Target: `fox-laptop`, Ryzen 7 6800U / Radeon 680M (REMBRANDT, RDNA2)

Workload: constrained-atlas Dungeon, 2880×1800 physical, Three r185 WebGPU

## Result

AMD Radeon GPU Profiler 2.7 identified one dominant graphics event in both the
base and POM captures: event 12, `vkCmdDrawIndexed(6, 1, 0, 0, 0)`. It shades
approximately 4.56 million pixels from two triangles into the 2880×1800
`VK_FORMAT_R16G16B16A16_SFLOAT` main target.

| Event 12 metric | Base | POM | POM delta |
|---|---:|---:|---:|
| Event duration | 4,824.465 µs | 5,748.746 µs | +924.281 µs (+19.2%) |
| Work duration | 4,291.628 µs | 5,090.940 µs | +799.312 µs (+18.6%) |
| Shaded pixels | 4,558,668 | 4,559,180 | +512 |
| PS wavefronts | 71,629 | 71,637 | +8 |
| FS VGPRs | 40 | 56 | +16 |
| FS SGPRs | 128 | 128 | 0 |
| FS scratch spills | none | none | none |
| Theoretical FS occupancy | 12/16 | 9/16 | -25% |

RGP explicitly reports vector-register usage as the occupancy limiter. The base
shader needs four fewer VGPRs to reach the next occupancy tier; the POM shader
needs eight fewer. The trace therefore identifies the full-coverage material
fragment draw as the event to inspect and shows POM's effect on shader resources.
It does **not** establish a 4.8–5.7 ms production cost: SQTT tracing perturbs
execution, and the safe immediate capture occurred before fine-page residency
settled.

RGP's render-target view reports the same traced main pass at 4,825.930 µs base
and 5,749.917 µs POM. Secondary 2880×1800 RGBA8 passes were 0.55–1.55 ms.

A subsequent non-traced, settled 2880×1800 ablation used Three's timestamp for
the latest main render context over 40 samples at the same forward pose:

| Non-POM material | Mean | p50 | Range |
|---|---:|---:|---:|
| Constant `MeshStandardNodeMaterial` | 0.876 ms | 0.830 ms | 0.708–1.324 ms |
| VT albedo + constant roughness/geometric normal | 1.072 ms | 1.083 ms | 1.025–1.132 ms |
| Full VT albedo + normal + packed roughness/AO | 1.050 ms | 1.047 ms | 1.039–1.116 ms |

The albedo/full ordering is within independent-launch clock noise, but both show
that the settled full non-POM main render costs about 1.05 ms and the complete
VT material adds roughly 0.2 ms over constant standard PBR—not 4.8 ms. The RADV
compiler dump still explains why the shader is structurally non-trivial: the
inlined full base shader contains 1,135 static machine instructions, 14 image
operations, and 44 branches versus 287/2/2 for the constant standard shader.
Those fallback paths are most expensive when regular pages are absent, as in
the immediate RGP capture.

## Evidence retained in the repository

`docs/benchmarks/rgp/` contains screenshots of:

- base/POM frame summaries;
- base/POM most-expensive-event tables;
- base/POM dominant fragment-stage resource and occupancy details;
- POM event timing;
- base render-target durations;
- RGP system information.

The raw RADV captures were 46 MiB (base) and 52 MiB (POM) and were intentionally
not committed. Their session SHA-256 values were:

```text
60149b89affcbe56de142c3e673e5493f8bc5053bc3a3d96ed4b35bf8cddf0e6  base.rgp
8049dc7306a40229c4d0f26c62bc1e68107d54191a46426088679da328476d11  pom.rgp
```

## Capture procedure

### 1. Preserve the validated Vulkan stack

Do not switch the laptop to the host Mesa stack. Launch through the project
`shell.nix`, which selects the validated Nix Vulkan loader and Mesa 25.3.4 RADV
ICD. The host Fedora Mesa 26.1.4 stack is known to crash CEF 149 during Skia
clears and is not valid evidence for Afterglow.

Confirm the normal Dungeon and WebGPU path before tracing. Never accept WebGL
fallback or a GPU-process restart.

### 2. Enable RADV's built-in RGP trace

Mesa documents `MESA_VK_TRACE=rgp` and `MESA_VK_TRACE_TRIGGER=<path>`. RADV
writes `.rgp` files to `/tmp`. The trigger is created only after the target
state is selected.

```sh
rm -f /tmp/afterglow-rgp-trigger /tmp/*.rgp
XA=$(ls /run/user/1000/.mutter-Xwaylandauth.* | head -1)
setsid env \
  DISPLAY=:0 \
  XAUTHORITY="$XA" \
  MESA_VK_TRACE=rgp \
  MESA_VK_TRACE_TRIGGER=/tmp/afterglow-rgp-trigger \
  nix-shell shell.nix --run \
  "./target/debug/examples/dungeon --ozone-platform=x11" \
  </dev/null >/tmp/dungeon-rgp.log 2>&1 &
```

Wait only until the CDP harness appears, then isolate the main render:

```sh
./target/release/latency-tool eval \
  '(()=>{const a=window.__afterglowDungeon;
    a.setProgrammatic(true);
    a.setFeedbackEnabled(false);
    a.setPomEnabled(true); // false for the matching base capture
    return a.pipelineTelemetry()})()' \
  127.0.0.1:9222

touch /tmp/afterglow-rgp-trigger
find /tmp -maxdepth 1 -name '*.rgp' -type f -ls
```

Wait two seconds after the file appears, copy it off the laptop, and terminate
the traced CEF process. Repeat in a fresh process with POM disabled.

### 3. Safety rule: never soak with RGP instrumentation enabled

A failed attempt left RADV tracing active while roughly 2,000 VT requests
streamed. After about 25 seconds the laptop became unresponsive and rebooted;
there was no persisted kernel OOM or amdgpu-reset signature. Immediate
single-frame captures succeeded twice.

Therefore:

- do not wait for a full VT-atlas settle under `MESA_VK_TRACE=rgp`;
- disable feedback immediately;
- trigger one frame, copy the trace, and terminate;
- use ordinary timestamp/rAF runs for steady-state timings and RGP only for
  event/resource attribution;
- keep raw captures out of git because each is tens of MiB.

This means the recorded RGP frame has warmed pipelines but limited fine-page
residency. It is valid for identifying the dominant event and comparing
base/POM geometry, shader resources, occupancy, and traced duration under the
same capture conditions. Its absolute event duration is not production timing,
and it is not steady-state atlas-memory traffic evidence.

### 4. Install and launch AMD's viewer on NixOS

`https://gpuopen.com/rdts-linux/` currently redirects directly to the Linux
Radeon Developer Tool Suite tarball. The validated archive contained Radeon GPU
Profiler 2.7.0.32.

The binary needs an FHS-like runtime library path on NixOS. A temporary
`mkShell` should include the C++ runtime, libpng, fontconfig, libxkbcommon,
libglvnd, freetype, zlib, X11/xcb, Xau/Xdmcp, SM/ICE, xcb-util, dbus, expat,
brotli, and bzip2. Prepend the suite's own `lib/` directory to
`LD_LIBRARY_PATH`, set `QT_QPA_PLATFORM=xcb`, and pass the capture path as the
only argument:

```sh
export LD_LIBRARY_PATH="$RDTS/lib:$LD_LIBRARY_PATH"
export QT_QPA_PLATFORM=xcb
"$RDTS/RadeonGPUProfiler" capture.rgp
```

RGP's Linux binary does not provide useful `--help`; it opens the GUI. For
repeatable agent extraction under XWayland, use `xdotool` to select views and
ImageMagick `import -window <id>` to retain evidence screenshots.

### 5. Extract the data consistently

1. **Overview → Frame summary:** record GPU-bound classification, capture frame,
   event count, and profiling overhead.
2. **Overview → Most expensive events:** record event ID, Vulkan command,
   duration, work duration, and shader stages.
3. **Overview → Render/depth targets:** record format, physical dimensions,
   draw count, compression, and pass duration.
4. **Events → Wavefront occupancy:** select the dominant event and record shaded
   pixels, waves, threads, VGPR, SGPR, spills, and occupancy.
5. **Events → Pipeline state → FS:** record RGP's stated limiting resource and
   the register reduction needed for the next occupancy tier.
6. Repeat on the matching feature-off capture and compare the same event, not
   only whole-frame summaries.

### 6. CEF/RADV interpretation caveats

- CEF's GPU process presents multiple Vulkan surfaces. RGP's “GPU-based frame”
  delimiter can therefore be shorter than a command-buffer event in the same
  capture. Use matching event and render-target durations for trace-local
  comparisons; do not treat the overview frame duration as Afterglow
  presentation time.
- SQTT/RGP instrumentation and pre-residency fallback traversal can materially
  inflate event duration. Production timing must come from a non-traced,
  settled timestamp-query run.
- These RADV captures report `N/A` API shader hashes and no instruction-timing
  data. RGP still provides event timing, wave counts, register use, spills, and
  occupancy, but not source/ISA hotspots or instruction-level stalls.
- RGP profiling overhead was 45.85–51.57 MiB and reports the 680M's 512 MiB
  visible-memory carve-out, 51.2 GB/s memory bandwidth, 2.2 GHz shader clock,
  and 800 MHz memory clock.

## Primary references

- Mesa environment variables (`MESA_VK_TRACE`, trigger/frame controls, RADV SPM
  counter configuration): https://docs.mesa3d.org/envvars.html
- AMD Radeon GPU Profiler manual: https://gpuopen.com/manuals/rgp_manual/
- AMD Radeon Developer Tool Suite for Linux: https://gpuopen.com/rdts-linux/
