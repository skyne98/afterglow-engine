# Fence caller identification

This run repeats the native paint capture with syscall stack traces.
It uses the NVIDIA RTX 3090, driver 595.99.02, and the niri Wayland session.
The driver closed only its own processes.

## Result

`syscalls.log:286446` records a 24.981 ms poll on `anon_inode:sync_file`.
Its stack contains this sequence, from caller to wait:

```text
afterglow_shell::present_surface
deno_webgpu::canvas::GPUCanvasContext::present
wgpu_core::global::Global::surface_present
wgpu_core::instance::Surface::present
wgpu_hal::vulkan::Queue::present
wgpu_hal::vulkan::swapchain::native::NativeSwapchain::present
libnvidia-glcore.so.595.99.02 [0xe8a128]
libnvidia-glcore.so.595.99.02 [0xd10709]
libnvidia-glcore.so.595.99.02 [0xa71d2c]
libnvidia-glcore.so.595.99.02 [0xa08da3]
libc::__poll
```

The wait occurs inside the NVIDIA presentation implementation, not an RPC call.
The source in `wgpu-hal` 29.0.4, `src/vulkan/swapchain/native.rs:581`, calls `queue_present` through the Vulkan function table.
The stack does not identify which GPU workload controls the fence.

The capture retained 30,505 host records with no missing span beginnings and two open boundary spans.
Pixel, source, sequence, and loss checks passed.
The largest captured presentation interval was 41.660 ms, with 41.470 ms in surface presentation and no overlapping RPC.
The window had focus, no recorded suspension, and unknown occlusion state.
No one-second interval occurred in the captured presentation records.

## Method and limits

The existing `scripts/test-native-paint-capture.ts --strace` driver supplied the workload and process cleanup.
A temporary PATH wrapper added these strace options before its `--` separator:

```text
-k --stack-trace-frame-limit=24 -e trace=poll,ppoll
```

The last trace filter replaced the driver filter, so this run contains only poll and ppoll syscalls.
The wrapper and launcher were removed after the run.
`AFTERGLOW_HOST_TRACE=1` enabled host state output.
The syscall limit remained 250,000, and stacks had a 24-frame limit.
Stack truncation is explicit in the raw trace.

`syscall-summary.json` contains 11,662 completed calls.
Its 279,958 omitted lines include stack frames and other non-syscall text, not an estimate of lost calls.
The summary lists the longest 20 calls.

Stack collection adds substantial overhead.
This run identifies callers, not application timing limits.
The syscall and DGTL clocks have no explicit mapping, so their longest intervals are not necessarily the same event.
