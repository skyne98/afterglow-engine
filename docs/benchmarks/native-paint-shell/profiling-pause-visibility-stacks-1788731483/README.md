# Presentation socket caller

## Conclusion

An inactive-workspace move can block native presentation while the NVIDIA driver waits for Wayland events.
The wait occurs on the main thread inside `GPUCanvasContext::present`, below `NativeSwapchain::present`.
Thus the same wait delays JavaScript, input dispatch, and main-thread RPC response processing.
It is not a diagnostics socket wait or proof of slow worker computation.

The [matching control experiment](../profiling-pause-visibility-controls-1788731282/README.md)
reproduced a 997.555 ms rAF interruption with no collector connected, no recording, and zero transmitted batches.
Its traced companion recorded 993.773–999.398 ms in surface presentation during four long intervals, without overlapping application RPC.
These experiments identify a reproducible cause of the one-second symptom.
They do not prove that every historical interruption had this cause.

## Stack evidence

The longest `ppoll` in this stack run took 471.576 ms.
The next took 369.814 ms on the same Unix socket.
See `syscalls.log:155791` for the first syscall and its stack:

```text
ppoll(fd=3, UNIX-STREAM, POLLIN, timeout=NULL) = 1  <0.471576>
  libc::ppoll
  libwayland-client::wl_display_poll
  libwayland-client::wl_display_dispatch_queue_timeout
  libwayland-client::wl_display_dispatch_queue
  libnvidia-glcore.so.595.99.02 + 0xd10463
  libnvidia-glcore.so.595.99.02 + 0xe8a128
  wgpu_hal::vulkan::swapchain::native::NativeSwapchain::present
  wgpu_hal::vulkan::Queue::present
  wgpu_core::Surface::present
  wgpu_core::Global::surface_present
  deno_webgpu::GPUCanvasContext::present
  afterglow_shell::present_surface
  afterglow_shell::op_try_present_surface
  V8
  afterglow_shell::App::window_event
  winit::EventLoop::pump_events
```

The original log retains library paths, offsets, and the 40-frame limit.
The syscall has no timeout and returns when the socket becomes readable.
The stack identifies the socket caller without an inference from its descriptor number.
This differs from the earlier short `sync_file` fence waits in the same driver presentation path.

## Method and checks

The visibility driver ran with `--strace` inside `shell.nix`.
A temporary executable added these options to the existing tracer command before its `--` separator:

```text
-k --stack-trace-frame-limit=40 -e trace=poll,ppoll
```

The original tracer options retain descriptor decoding, syscall durations, bounded calls, and child cleanup.
The temporary executable and its directory were removed after the run.
The driver moved only its verified child window to an empty inactive workspace, then restored it.
The shell binary and presentation policy did not change.
Paint checks passed with 954 colored pixels in all three workloads.
The capture had no failures or lost records.

Files:

- `syscalls.log`: original syscall and stack data.
- `syscall-summary.json`: longest calls, counts, and omitted stack-line count.
- `visibility.json`: workspace actions and error result.
- `result.json`, `host.log`, `analysis.json`, and the DGTL file: application measurements and checks.

## Remaining limits

Stack collection changes timing substantially.
This run did not reproduce a full one-second wait, although the preceding control and syscall-only runs did.
The clocks still have no explicit common mapping.
The exact Wayland event or internal NVIDIA condition necessary to release the wait is not identified.
No background rendering policy, presentation mode, or event-loop change is part of this investigation.
Foreground frame-budget checks, sustained capture overhead, physical input latency, and soaks remain open.
