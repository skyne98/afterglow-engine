# Initial compositor-pacing check

The release shell uses `Window::pre_present_notify()` and independent runtime deadlines.
The test moved only its owned window to an inactive niri workspace for 4.5 seconds.
It then restored that window without a focus change.

| Measurement | Without capture | With capture |
|---|---:|---:|
| Maximum hidden-phase rAF interval | 1008.486 ms | 999.131 ms |
| Maximum 25 ms timer lateness, full measurement | 28.046 ms | 22.679 ms |
| Timer callbacks | 778 | 779 |
| Timer sample overflow | 0 | 0 |
| Painted pixels in each phase | 954 | 954 |

The compositor still supplies approximately one frame per second on the inactive workspace.
Those intervals no longer mean that the main thread waits inside presentation.
The four longest captured intervals contain only 1.686–5.101 ms of surface presentation each.
The maximum surface presentation duration across all 1,485 captured spans was 27.352 ms.
The independent timer continued between frames.
The capture retained 22,377 host records, with no missing span beginnings and two open endings.

## Evidence

- `control/result.json` and `capture/result.json`: rAF phases, fixed timer samples, pixels, and capture status.
- `control/visibility.json` and `capture/visibility.json`: owned window movement.
- `capture/analysis.json`: host interval and stage analysis.
- `capture/presentation.json`: all captured surface presentation spans.
- Each directory also contains its host log. The capture directory retains the DGTL file.

## Limits

This is one paired test, not the complete repeated-cycle or soak gate.
Timer lateness is not OS input-to-pixel latency.
The observer and host clocks do not have an explicit mapping.
The native menu failure reported after this change remains a separate open regression.
