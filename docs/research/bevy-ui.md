# Bevy 0.18 UI System — Deep Dive

## Architecture

```
ECS World (Main)
  └─ UiSystems (Prepare → Propagate → Content → Layout → PostLayout → Stack)
       ├─ propagate_ui_target_cameras
       ├─ measure_text_system
       ├─ update_image_content_size_system
       ├─ ui_layout_system (Taffy flexbox)
       ├─ text_system (Cosmic Text glyph layout)
       └─ ui_stack_system (z-order)

ExtractSchedule
  └─ extract_uinode_backgrounds/images/borders/text/...

Render World
  ├─ queue_uinodes → TransparentUi phase items
  ├─ sort by z_order
  └─ prepare_uinodes → vertex/index buffers, batch by image

Render Graph (Core2d/Core3d)
  └─ EndMainPass → UiPass
       └─ DrawUi (SetPipeline, SetViewBindGroup, SetTextureBindGroup, DrawIndexed)
```

## Core Components

Every UI entity requires `Node`, which auto-adds 7 others:

| Component | Required? | Purpose |
|---|---|---|
| `Node` | Yes | All CSS-like style properties (flexbox, sizing, positioning) |
| `ComputedNode` | Auto | Layout results: size, border, radius, scroll, outline |
| `ComputedUiTargetCamera` | Auto | Which camera renders this node |
| `ComputedUiRenderTargetInfo` | Auto | Scale factor + render target size |
| `BackgroundColor` | Auto | Background fill color |
| `BorderColor` | Auto | Per-side border colors |
| `FocusPolicy` | Auto | `Block` or `Pass` for interaction |
| `ScrollPosition` | Auto | Scroll offset |
| `Visibility` | Auto | Inherited hierarchy visibility |
| `ZIndex` | Auto | Local z-ordering among siblings |

## Layout (Taffy Flexbox)

Bevy uses **Taffy** for layout. `ui_layout_system` runs in `UiSystems::Layout`:
1. **Sync** — converts `Node` styles to Taffy, upserts into tree
2. **Children** — updates parent-child relationships
3. **Compute** — calls `taffy.compute_layout()` per UI root
4. **Geometry** — reads results, writes `ComputedNode`, `UiGlobalTransform`

### Style Properties

All standard CSS flexbox: `flex_direction`, `justify_content`, `align_items`, `flex_grow/shrink/basis`, `gap`, `margin`, `padding`, `position_type`, `overflow`, `grid_*`, `aspect_ratio`.

### Val Types

`Val::Px(f32)`, `Percent(f32)`, `Vw(f32)`, `Vh(f32)`, `VMin(f32)`, `VMax(f32)`, `Auto`.

### Measure

- `FixedMeasure` — constant size
- `TextMeasure` — Cosmic Text based
- `ImageMeasure` — intrinsic image size
- `Custom(Box<dyn Measure>)` — user-defined

## Text

`Text` component requires `Node`, `TextFont`, `TextColor`, `TextLayout`, `LineHeight`, `ContentSize`.

- Uses **Cosmic Text** for layout/shaping
- Text spans via child entities with `TextSpan`
- `TextLayoutInfo` contains positioned glyphs
- `TextShadow` for drop shadows
- `Strikethrough`, `Underline` decorations
- `TextBackgroundColor` per-text-run backgrounds

## Images

`ImageNode { color, image, texture_atlas, flip_x/y, rect, image_mode }`

Image modes: `Auto` (intrinsic size), `Stretch`, `Sliced(TextureSlicer)` (9-slice), `Tiled`.

## Interaction

- **`Interaction`**: `Pressed` | `Hovered` | `None`
- **`FocusPolicy`**: `Block` (captures clicks) | `Pass` (lets through)
- **`ui_focus_system`** — walks `UiStack` top-down, SDF hit-test, respects clipping
- **`UiPickingPlugin`** — integration with `bevy_picking` for advanced picking

## Z-Ordering

| System | Scope |
|---|---|
| `GlobalZIndex` | Global ordering across all roots |
| `ZIndex` | Local ordering among siblings |
| `UiStack` | Sequential stack index per node |
| `stack_z_offsets` | Micro-offsets within a node: background 0.0, border 0.01, image 0.04, text 0.06 |

## Overflow & Scrolling

`Overflow { x: OverflowAxis, y: OverflowAxis }`:
- `Visible` — no clip
- `Clip` — clipped, no scroll
- `Hidden` — clipped + layout treats overflow as zero size
- `Scroll` — clipped with scrollbars

`ScrollPosition` stores scroll offset. `IgnoreScroll` per-axis control.

## 9-Slice Rendering

Separate pipeline (`UiTextureSlicePipeline`) with its own vertex format (`UiTextureSliceVertex`). Handles sliced and tiled images. 1.25× scale adjustment for seamless borders.

## Border & Outline

`BorderColor` supports per-side colors. Sides with the same color merge into one draw call. `Outline` renders as a border with configurable width and offset. Rounded corners via SDF in the shader.

## UiScale & Resolution

`UiScale(f32)` — global multiplier (default 1.0). Per-camera DPI via camera's `scale_factor`. Combined to produce `ComputedUiRenderTargetInfo`.

## Known Limitations

- **Borders on leaf nodes** — may not render correctly (TODO in convert.rs)
- **Elliptical border radius** — not supported
- **No inline text styling** — requires span entities
- **Ghost nodes** — experimental (feature-gated)
- **No text selection/caret** — manual implementation needed
- **UiScale is global** — not per-camera
- **Single-threaded layout** — can be a bottleneck for complex UIs

## Key Examples

| Example | What it shows |
|---|---|
| `button.rs` | Button with interaction states |
| `text.rs` | Text with multiple styled spans |
| `text_decorations.rs` | Underline, strikethrough |
| `grid.rs` | CSS Grid layout |
| `scroll.rs` | Scroll containers |
| `overflow.rs` | Clip vs scroll vs visible |
| `9_slice.rs` | 9-sliced image scaling |
| `viewport_nodes.rs` | Render-to-texture UI |
| `tab_navigation.rs` | Keyboard navigation |
| `directional_navigation.rs` | Gamepad/d-pad navigation |

## References

- Source: `bevy_ui-0.18.1/src/`, `bevy_ui_render-0.18.1/src/`
- Taffy: https://github.com/DioxusLabs/taffy
- Examples: `examples/ui/` in bevy repository
