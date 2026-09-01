# Wide paint stroke wake audit — 2026-08-24

## Configuration

The test used these items:

- AMD Ryzen 9 9950X3D with 32 logical CPUs
- Chromium 146 with cross-origin isolation
- The character-editor Vite development server
- A 16,384 x 16,384 document
- A 9,830-pixel test stroke with 10 input samples
- The Tail Feathers brush with radius 60

A page `MutationObserver` measured the time from the test-stroke command to a zero motion queue. A temporary diagnostic build counted dabs and continuation batches.

## Cause

The Tail Feathers test made 79,284 dabs and 613 continuation batches. Each batch processed a maximum of 128 dabs.

The worker used nested `setTimeout(0)` calls to start the next continuation. Chromium applied an approximately 4 ms timer clamp after the first nested calls.

Thus, the timer delay was approximately 2.45 seconds for 613 continuations. Most of the delay was not brush or tile work.

A separate 613-wake Worker test measured these values:

| Wake mechanism | Total time |
|---|---:|
| Nested `setTimeout(0)` | 2,472.125 ms |
| Fixed `MessageChannel` | 1.095 ms |

Sparse input made the problem easy to see. One long input segment needed many 128-dab continuations, but many short local segments could share one tile batch.

The classic Brush test gave this evidence:

| Input | Dabs | Batches | Total time |
|---|---:|---:|---:|
| 10 wide samples | 4,613 | 33 | 382.885 ms |
| 100 local samples | 4,529 | 1 | 131.425 ms |

The dab totals were almost equal. The batch count and wake delay caused the large difference.

## Correction

The worker now uses one fixed `MessageChannel` wake. It permits only one pending drain wake and still gives one worker task between continuation batches.

A later queue audit found that several 128-dab continuation units in one tile batch could exceed the fixed operation queue. The worker now processes one input sample or continuation unit per batch. This adds fixed wake work, but preserves the queue limit and input order.

The change does not change these items:

- The 128-dab work limit
- The fixed operation capacity
- The fixed motion and command capacities
- Brush command order
- Brush output

Five final Tail Feathers runs measured a 47.8 ms median. The range was 45.090 through 70.530 ms.

The prior path measured 2,516.7 through 2,524.8 ms. Thus, the median improvement was approximately 52 times.

The final classic Brush wide test measured 156.975 ms. Its remaining difference from local input is tile-batch, pthread, and display work, not timer clamp. These timing values predate the one-sample batch correction.
