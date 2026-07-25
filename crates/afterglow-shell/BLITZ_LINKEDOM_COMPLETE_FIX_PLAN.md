# Complete Blitz ↔ LinkeDOM Repair Plan

## 1. Objective

Replace the current serialization-based prototype with one coherent browser subsystem in which:

- LinkeDOM is the only JavaScript-facing DOM and owns JavaScript object identity, DOM structure, attributes, text, event listeners, and script-visible mutable state.
- Blitz owns CSS parsing/cascade, computed values, layout, box metrics, paint ordering, hit testing, focus render state, scrolling geometry, and page composition.
- Every transition between the two has a typed protocol, one owner, a defined flush point, and a tested browser-visible result.
- Client examples and three.js addons run byte-for-byte unmodified.

This is an **atomic replacement**, not a staged product rollout. The implementation may be developed in dependency order, but no partial bridge, compatibility fallback, feature flag, example-specific branch, injected client patch, or temporary stub is accepted in the final tree.

The work explicitly excludes unrelated WebGPU numerical differences, temporal frame selection, and video decoding. It includes every presently known DOM/CSS/layout/interaction/composition problem caused by the Blitz/LinkeDOM boundary.

## 2. Non-negotiable invariants

1. Never edit `examples/*.html`, extracted module source, or `three/addons/*`.
2. Never replace an addon with a stub.
3. Never hide a missing browser behavior with transport CSS or example detection.
4. Never expose transport IDs as HTML attributes, even temporarily.
5. Never maintain two independently mutable DOM trees. LinkeDOM sends source-state changes; Blitz returns only derived state and transactional user-agent actions.
6. Every synchronous style/layout getter flushes all prior LinkeDOM and CSSOM mutations before querying Blitz.
7. Every native default action dispatches cancelable JavaScript events before committing native state.
8. Canvas backing dimensions and CSS box dimensions remain separate.
9. Full page capture always follows the same Blitz paint tree used by geometry and hit testing.
10. A bridge inconsistency is an error containing epoch, native node ID, operation, and expected state; it is never converted to zero dimensions or a no-op.
11. Diagnostics are removed after the corresponding defect is fixed.
12. One unconditional implementation is used by all 182 examples.

## 3. Current implementation to replace

The following are prototypes and must disappear:

- `outerHTML` transport and reparsing on every dirty epoch.
- Temporary `data-three-native-node` attributes.
- Temporary canvas `width`/`height` attribute edits.
- Renaming stylesheet links during serialization.
- Injected `html, body { min-* }` and `canvas { display:inline-block }` rules.
- Inline-style `getComputedStyle()` proxy.
- Canvas-only `clientWidth`/`clientHeight` overrides.
- Fullscreen-canvas raw-buffer shortcut and visible-text heuristic.
- Direct mutation of Blitz `special_data` followed by ad hoc relayout calls.
- White alpha flattening not derived from browser screenshot semantics.
- Rebuilding the Blitz node map after every document parse.

## 4. Final source layout

Split the current `src/browser.rs` into explicit concerns:

```text
src/browser/
  mod.rs                 BrowserRuntime orchestration and lifecycle
  protocol.rs            serde request/response types and versioning
  document.rs            stable native-ID ↔ Blitz-ID reconciliation
  mutations.rs           mutation validation and ordered application
  state.rs               form/focus/scroll/selection/live property state
  style.rs               computed-style and CSSOM adapter
  resources.rs           deterministic Blitz NetProvider and readiness
  geometry.rs            DOMRect and all box metric queries
  interaction.rs         hit test, focus, pointer, scroll, default actions
  observers.rs           ResizeObserver/IntersectionObserver collection
  canvas.rs              GPU/2D canvas registry and raster ownership
  paint.rs               exact canvas primitive and final compositor
  fonts.rs               deterministic font collection and aliases
  error.rs               structured bridge failures
```

JavaScript bridge code moves out of the monolithic `dom_setup.ts`:

```text
browser_bridge/
  bootstrap.js           installation and lifecycle
  ids.js                 WeakMap native identities for every Node
  journal.js             DOM mutation journal and synchronous flush
  cssom.js               stylesheet/declaration mutation journal
  geometry.js            DOMRect/box metric APIs
  computed_style.js      read-only live CSSStyleDeclaration facade
  interaction.js         focus/scroll/hit-test/default-action integration
  observers.js           observer registration and delivery
  canvas.js              canvas binding and backing/CSS-size reflection
```

`dom_setup.ts` imports or embeds the built bridge bundle but no longer implements layout approximations.

## 5. Pin and repair Blitz at the correct layer

### 5.1 Reproducible dependency ownership

Vendor the exact Blitz monorepo revision under the workspace's `vendor/afterglow-shell-blitz/` and use root `[patch.crates-io]` path entries for every changed crate (`blitz-dom`, `blitz-html`, and `blitz-paint`). Preserve upstream licenses and record the upstream commit. Do not edit Cargo registry files. Keep AnyRender/Vello versions exactly pinned in `Cargo.lock`.

Each local Blitz change requires:

- a minimal upstream-style regression test;
- a comment citing the browser behavior being implemented;
- no three.js example names in the implementation;
- a separately reviewable patch suitable for upstreaming.

### 5.2 Inline replaced elements

Fix default-inline `<canvas>` as an atomic inline replaced element instead of changing its display type.

Required behavior:

- default intrinsic canvas size is 300×150 CSS pixels;
- reflected width/height content attributes change intrinsic size;
- CSS width/height override the used box without changing backing size;
- `auto` dimension plus aspect ratio derives from intrinsic ratio;
- min/max constraints apply after intrinsic contribution;
- line box ascent/descent and baseline match an inline replaced element;
- `display:block`, `inline-block`, absolute positioning, transforms, and flex/grid participation use the same replaced context;
- zero backing dimensions are allowed without accidentally deleting the CSS box.

Extend Blitz `CanvasData`/replaced context to carry intrinsic dimensions and an external raster handle. Do not swap canvas nodes to image nodes to obtain measurement.

### 5.3 Initial containing block

Fix percentage sizing for absolutely/fixed positioned descendants whose containing block is the initial containing block:

- viewport establishes the initial containing block;
- `left/right/top/bottom` and percentage width/height resolve against it when no positioned ancestor exists;
- root/body canvas background propagation follows CSS canvas rules;
- body content may exceed the viewport without changing a `height:100%` initial-containing-block result;
- fixed positioning remains viewport-relative under document scroll.

Remove both injected `html/body` minimum-size declarations after these tests pass.

### 5.4 Public derived-state APIs

Add narrow Blitz APIs instead of reading internal fields from runtime code:

- `computed_property(node, pseudo, PropertyId) -> String` using Stylo `computed_or_resolved_value`;
- `computed_custom_properties(node) -> iterator`;
- `box_metrics(node) -> BoxMetrics` containing border/client/padding/content boxes, scroll extent, transformed client rect, offset parent, and integer browser metrics;
- `client_rects(node) -> Vec<Rect>` for inline fragments;
- `hit_stack(x, y) -> Vec<node_id>` in front-to-back paint order;
- focus and scroll state getters/setters;
- a canvas external-raster registration API that invalidates only paint when dimensions are unchanged and layout+paint when intrinsic dimensions change.

### 5.5 Exact canvas paint command

Add a Blitz Paint display command for external canvas raster content. It must preserve source bytes when all of these are true: integer-aligned 1:1 placement, identity color space, opacity 1, rectangular clip, and source-over composition onto an equivalent destination. For scaling/transforms/opacity, use the normal renderer with explicitly selected browser-equivalent sampling and premultiplied-alpha rules.

This replaces the fullscreen shortcut while preserving byte identity for ordinary fullscreen canvases and still painting arbitrary DOM layers above and below them.

## 6. Typed bridge protocol

### 6.1 Identity

Use JavaScript `WeakMap<Node, bigint>` and a monotonically increasing 64-bit ID. Assign IDs to Document, DocumentType, Element, Text, and Comment nodes. IDs are never placed on DOM objects or in attributes. A clone receives new IDs naturally. A moved node retains its ID. Removed nodes remain in the native slab as disconnected nodes until document teardown; do not rely on nondeterministic garbage-collection/finalization notifications. If a bridge API is first called on a detached node that has never been connected, serialize that detached subtree into the native slab before servicing the query.

Reserve ID 0 as invalid. Detect wraparound. Rust stores both maps:

```rust
HashMap<NativeNodeId, BlitzNodeId>
HashMap<BlitzNodeId, NativeNodeId>
```

### 6.2 Initial snapshot

Send one structured initial snapshot, not HTML:

```rust
DocumentSnapshot {
    protocol_version,
    epoch,
    document_id,
    quirks_mode,
    url,
    viewport,
    nodes: Vec<NodeRecord>,
    resources: Vec<ResourceDescriptor>,
    live_state: Vec<LiveStateRecord>,
}

NodeRecord {
    id,
    parent,
    previous_sibling,
    kind: Document | Doctype | Element | Text | Comment,
    namespace,
    prefix,
    local_name,
    attributes: Vec<QualifiedAttribute>,
    text,
}
```

Validate unique IDs, one document root, parent/sibling consistency, acyclic structure, namespaces, and epoch ordering before changing Blitz. Build a `BaseDocument` directly through `DocumentMutator`; add a small upstream builder API if required rather than parsing synthetic HTML.

### 6.3 Ordered mutation batches

After bootstrap, send ordered batches:

```rust
MutationBatch {
    protocol_version,
    base_epoch,
    target_epoch,
    operations: Vec<Mutation>,
    live_state: Vec<LiveStateDelta>,
    cssom: Vec<CssomMutation>,
}
```

Supported mutations:

- `CreateNode(NodeRecordWithoutPosition)`
- `InsertBefore { parent, node, reference }`
- `Remove { parent, node }`
- `SetAttribute { node, qualified_name, value }`
- `RemoveAttribute { node, qualified_name }`
- `SetText { node, value }`
- `RegisterDetachedSubtree { nodes }` for first native access to never-connected nodes
- document URL/base/quirks changes

MutationObserver records mark and populate the journal. `takeRecords()` is consumed synchronously before every native query. Added subtrees are serialized recursively once; moves use the same IDs. `innerHTML`, fragments, cloning, and `replaceChildren()` therefore reduce to ordinary creates/removes/inserts without source rewriting.

Apply a batch transactionally:

1. validate epoch and all referenced IDs;
2. validate the resulting tree relationships without mutating Blitz;
3. apply operations in order through one `DocumentMutator` lifetime;
4. update style/resource registrations generated by the batch;
5. apply live state;
6. call one resolve pass appropriate to accumulated damage;
7. commit epoch and return an acknowledgment containing style/layout/paint generations.

On failure, reject the whole batch and include the operation index. In debug tests, compare a canonical JS tree digest with a canonical native tree digest after every flush.

### 6.4 CSSOM journal

MutationObserver does not observe all CSSOM operations. Wrap the environment’s actual CSSOM entry points, without changing client code:

- `CSSStyleSheet.insertRule/deleteRule/replace/replaceSync`
- `CSSStyleDeclaration.setProperty/removeProperty` when attached to a rule
- adopted stylesheet changes if LinkeDOM exposes them
- `<style>` text changes remain ordinary DOM mutations

CSSOM records identify owner sheet/node, rule index/path, operation, and exact source text. Native style sheets are updated through Stylo APIs and invalidate style/layout correctly. Add differential tests for all operations.

## 7. Deterministic resource system

### 7.1 One resource registry

Create a shared Rust `BrowserResourceRegistry` used by JavaScript fetch/image/font code and Blitz `NetProvider`. It resolves URLs against the document/base URL and returns the exact same bytes, MIME type, cache key, and failure to both sides.

Resource classes:

- external stylesheets and nested `@import`;
- CSS background/list/mask images;
- `<img>` and SVG resources;
- `@font-face` sources;
- data/blob/file/HTTP URLs already supported by the runtime;
- dynamically inserted links and changed `href`/`media`/`disabled` state.

### 7.2 Stylesheet lifecycle

Do not rename `<link>` elements and do not inject `main.css`. Preserve actual nodes and attributes. Blitz requests the resource through the registry, parses it with its real URL as the stylesheet base, and reports load/error completion.

Track:

- request generation;
- pending/ready/failed state;
- parent `@import` dependency graph;
- media applicability;
- sheet order and disabled state;
- style/layout invalidation when bytes arrive.

Before module execution, await parser-blocking/initial critical styles used by the loaded document. Dynamic resources participate in the host event-loop readiness mechanism and dispatch LinkeDOM `load`/`error` events. A synchronous layout read while a relevant resource is pending pumps already-available deterministic completions; it must not silently lay out without the stylesheet.

### 7.3 Fonts

Use Chromium CDP `CSS.getPlatformFontsForNode` against the reference environment to identify the actual generic font faces used by `main.css` and overlays. Vendor those exact font files and register deterministic family aliases, weights, and styles in Blitz. Test family fallback, missing glyphs, and font-load invalidation.

First target metric parity. Then compare isolated glyph rasters. If Vello/Parley cannot meet the strict screenshot threshold with identical fonts and hinting settings, add a pinned Skia CPU text paint backend (or a Skia-backed AnyRender text command) rather than compensating layout or changing client CSS.

## 8. Browser lifecycle and flush model

Maintain generations:

- DOM mutation epoch;
- CSSOM epoch;
- resource epoch;
- style generation;
- layout generation;
- paint generation;
- viewport/environment generation.

Define one JavaScript `flushBrowser(reason)` function:

1. prevent recursive entry and report the initiating API on recursion;
2. consume pending MutationObserver records;
3. finalize ordered DOM and CSSOM journal entries;
4. send a batch only when source state changed;
5. allow native deterministic resource completions;
6. resolve style and layout exactly once if required;
7. receive derived-state/action/observer records;
8. cache generation numbers, never derived values past their generation.

Flush before:

- computed style reads;
- all geometry/scroll metrics;
- hit tests;
- focus navigation requiring layout;
- observer delivery checkpoints;
- canvas CSS-size reads used by renderer resizing;
- final capture.

Do not force a new epoch merely because capture was requested. A no-change flush must be idempotent and return the same layout/paint hashes.

## 9. Computed style implementation

Implement `getComputedStyle(element, pseudo)` as a read-only, live JavaScript `CSSStyleDeclaration` facade.

Behavior:

- validates the Element and supported pseudo selector, while reproducing browser behavior for both connected and detached elements;
- flushes before each observable read unless generations prove the object current;
- obtains values from Stylo, never `element.style`;
- supports kebab-case `getPropertyValue`, camel-case named properties, indexed access, `length`, and `item()`;
- includes custom properties;
- serializes colors, lengths, transforms, font values, display, position, opacity, z-index, and shorthands according to CSSOM expectations;
- throws or ignores writes exactly as a read-only declaration requires;
- supports `::before` and `::after` once Blitz exposes pseudo computed values;
- keeps returned objects live across subsequent mutations.

Use a batched native query for enumeration and a single-property query for lazy named access. Cache by `(node, pseudo, style_generation, layout_generation)` because resolved used values may depend on layout.

Delete the current Proxy entirely.

## 10. Geometry and scrolling APIs

Return typed native structures for:

- transformed `getBoundingClientRect()`;
- fragment-aware `getClientRects()`;
- border, padding, content, and scroll boxes;
- offset parent and offset coordinates;
- client and scroll dimensions;
- current element and viewport scroll offsets.

JavaScript exposes real `DOMRectReadOnly`, mutable `DOMRect` where appropriate, and `DOMRectList` behavior. Implement browser integer rounding for `client*`, `offset*`, and `scroll*`; retain doubles for DOMRect values.

Cover:

- disconnected/display-none nodes;
- inline fragments and empty inline boxes;
- transforms and nested scroll containers;
- borders and scrollbars;
- writing modes;
- fixed/absolute/sticky positioning;
- root/body special cases;
- SVG geometry where Blitz supports it.

Implement `scroll()`, `scrollTo()`, `scrollBy()`, `scrollIntoView()`, and property setters by sending explicit native commands. Native clamps against Blitz scroll extents, updates render state, and queues LinkeDOM `scroll`/`scrollend` events. Preserve offsets across unrelated DOM mutations.

## 11. Interaction and default-action transactions

### 11.1 Hit testing

Expose Blitz’s complete front-to-back hit stack, accounting for transforms, clips, visibility, `pointer-events`, scrolling, and stacking contexts. Implement `document.elementFromPoint()` and `elementsFromPoint()` by mapping native IDs back to the existing LinkeDOM objects.

### 11.2 Pointer and mouse events

For each host input:

1. flush layout;
2. hit-test in Blitz;
3. construct pointer/mouse events in LinkeDOM with viewport/page/offset coordinates;
4. dispatch capture/target/bubble phases through LinkeDOM;
5. report cancellation and listener-induced DOM mutations;
6. flush those mutations;
7. commit or reject the proposed native default action;
8. queue resulting focus, scroll, input/change, hover, and observer effects.

Implement pointer capture, related targets, enter/leave boundaries, button state, modifiers, wheel deltas, and cancelation. Native state may never dispatch JavaScript callbacks directly while Rust holds a mutable document borrow.

### 11.3 Focus and keyboard

Implement `focus()`, `blur()`, `activeElement`, autofocus, tab order, disabled/inert filtering, and focus-visible/focus-within state. Dispatch `blur`, `focusout`, `focus`, and `focusin` in browser order. Keyboard Tab uses Blitz focus order; Enter/Space proposes element-specific activation.

### 11.4 Form defaults

Implement transactional browser defaults for controls Blitz supports:

- checkbox/radio checkedness and radio groups;
- option/select selection;
- labels activating associated controls;
- buttons;
- input value and selection state;
- disabled/read-only behavior;
- `input` and `change` event ordering.

Mirror committed live property state back into LinkeDOM properties without fabricating content-attribute changes unless the browser would reflect them.

## 12. Observers and media environment

### 12.1 ResizeObserver

Store native observation registrations by node ID and box type. After layout, compare canonical box sizes against the last delivered generation. Queue entries and deliver them in JavaScript’s observer checkpoint, including loop detection and the resize-loop error rule.

### 12.2 IntersectionObserver

Compute intersections from Blitz transformed geometry, ancestor clipping, root/rootMargin, thresholds, and viewport scroll. Queue entries only on threshold crossings and deliver with deterministic timestamps.

### 12.3 Media queries

Replace handwritten `matchMedia()` parsing with Stylo media evaluation using the same device as layout. Expose a native `evaluate_media(query)` API and register query dependencies. Viewport, DPR, color scheme, reduced motion, pointer/hover capability, and media type changes invalidate Stylo and queue MediaQueryList `change` events.

## 13. Canvas ownership and composition

### 13.1 Canvas state

Bind every JS canvas node ID to its GPU or 2D backing object. Track independently:

- backing width/height;
- reflected content attributes;
- CSS intrinsic width/height and ratio;
- context type and alpha mode;
- latest raster generation;
- premultiplication and color-space metadata.

Canvas property setters use normal reflected-attribute semantics and journal the resulting attribute mutation. They also reset backing-store/context state as browsers do. No temporary attributes are created during synchronization.

### 13.2 Raster handoff

At paint/capture, read only canvas generations needed by the Blitz display list. Convert GPU readback from its declared premultiplication/color space exactly once into the compositor’s declared input representation. Register the raster through the Blitz canvas API; never rewrite `SpecialElementData` directly.

### 13.3 Final paint

Paint one 800×500 CSS viewport using Blitz’s resolved display tree:

- CSS canvas/root/body background propagation;
- DOM backgrounds and borders;
- negative and positive stacking levels;
- canvas commands in exact paint position;
- text/images/forms above or below canvases;
- opacity groups, transforms, clips, border radii, and overflow;
- scroll offsets and fixed elements.

The output contract is opaque sRGB RGBA equivalent to Chrome screenshot capture. Transparent intermediate pixels are composited against the resolved CSS canvas background, falling back to the user-agent screenshot background only when the CSS canvas remains transparent.

Delete the fullscreen raw-buffer path after exact-canvas paint tests pass.

## 14. Test program

### 14.1 Protocol/reconciliation unit tests

Test initial construction and every mutation, including:

- insert/remove/move/reinsert;
- clone and fragment insertion;
- `innerHTML`, `outerHTML`, `replaceChildren`, text content;
- attribute namespaces, class/id/style changes;
- detached subtrees and later reconnection;
- templates, comments, doctypes, SVG/MathML namespaces;
- live form state independent of attributes;
- malformed/out-of-order/duplicate-ID batches rejected transactionally;
- canonical JS/native digest equality.

### 14.2 Chrome differential fixtures

Add dedicated test fixtures outside `examples/`; never modify canonical three.js clients. Run each fixture both in Chrome and the runtime and compare JSON results for:

- computed values and custom properties;
- block/inline/replaced/flex/grid/intrinsic layout;
- percentages, viewport units, absolute/fixed/sticky positioning;
- transforms, clipping, overflow, scrolling;
- client/offset/scroll metrics and rounding;
- hit stack and event order;
- focus/default actions;
- ResizeObserver/IntersectionObserver/media changes;
- dynamic stylesheet and resource behavior.

Use tolerances only where the web platform explicitly permits variation. Box values expected to match Chrome exactly are asserted exactly.

### 14.3 Paint tests

Create minimal pixel fixtures for:

- opaque fullscreen canvas byte identity;
- transparent canvas over body color/image;
- DOM below and above an opaque canvas;
- non-text overlay above canvas, proving removal of the old heuristic;
- multiple overlapping canvases;
- scaled/transformed/clipped canvas;
- opacity and nested stacking contexts;
- text baseline next to inline canvas;
- root/body background propagation;
- device scale and fractional placement.

Compare against Chrome PNGs with per-pixel diagnostics and classify every mismatch by display command, not by visual guess.

### 14.4 Existing target set

Run these after every bridge milestone during development, but do not merge a partial cutover:

- `webgpu_multiple_canvas`
- `webgpu_multiple_elements`
- `webgpu_centroid_sampling`
- `webgpu_loader_texture_ktx2`
- `webgpu_textures_anisotropy`
- `webgpu_hdr`
- `webgpu_compile_async`
- `webgpu_camera_logarithmicdepthbuffer`
- `webgpu_clipping`
- `webgpu_water`
- `webgpu_materials_basic`

Record image hash and diff percentage for at least three repeat runs.

### 14.5 Full-suite gate

Run all 182 examples serially on the intended deterministic backend in a runner that reuses a controlled process/device lifecycle or otherwise proves adapter availability. Then run selected NVIDIA checks. Produce:

- strict pass count;
- runtime errors;
- deterministic repeat hashes;
- before/after table;
- list of residual failures classified as bridge, paint/font, WebGPU numerical, temporal, or missing non-DOM API.

A backend adapter exhaustion is an infrastructure failure, not a test result.

## 15. Atomic cutover sequence

The implementation order below is for dependency management only. The product path changes once, after all pieces pass their dedicated tests.

1. Capture baseline images, geometry JSON, hashes, and current 140-pass manifest.
2. Vendor/pin Blitz and land upstream-style tests for inline canvas, initial containing block, computed values, box metrics, and exact canvas paint.
3. Introduce typed Rust protocol and direct `BaseDocument` builder under tests.
4. Implement JS WeakMap IDs, structured initial snapshot, mutation journal, CSSOM journal, and digest tests.
5. Implement the deterministic resource registry and make Blitz consume real links/imports/images/fonts.
6. Implement live state, computed style, geometry, scrolling, media, and observer APIs against the test-only new bridge.
7. Implement hit testing, focus, pointer dispatch, and default-action transactions.
8. Implement canvas external-raster ownership and exact composition; finish font matching/text backend.
9. Run all differential fixtures and targeted examples with the new bridge explicitly invoked by test harness code—not by modifying examples.
10. In one cutover change, replace the prototype bridge and compositor unconditionally.
11. Delete old serialization, transport markers, CSS injection, fake computed style, metric overrides, direct canvas special-data writes, and fullscreen shortcut in the same change.
12. Run format, locked build, Rust/JS tests, targeted tests, repeat determinism tests, and full 182-example suite.
13. Remove instrumentation and assert source searches find no forbidden paths or compatibility fallbacks.
14. Update `README.md`, architecture contract, failure classification, and knowledge base with final measured results.

## 16. Acceptance criteria

The work is complete only when all conditions hold:

### Architecture

- One long-lived Blitz `BaseDocument` exists per LinkeDOM document.
- No HTML serialization/reparse occurs after bootstrap.
- No transport data appears in HTML attributes or selectors.
- No injected CSS compensates for layout-engine defects.
- Real linked/dynamic stylesheets and CSS resources load through one provider.
- All computed-style and geometry reads come from Blitz.
- Hit testing, focus, scrolling, observers, and media use Blitz-derived state.
- Canvas content is a first-class Blitz paint source.

### Correctness

- Mutation and CSSOM differential tests pass.
- Computed style and geometry fixtures match Chrome.
- Inline canvas and initial-containing-block tests pass without transport rules.
- Non-text overlays compose correctly, proving the old shortcut is gone.
- Opaque 1:1 canvases retain byte identity through the complete compositor.
- Alpha/color-space fixtures match Chrome.
- Event/default-action order and cancelation tests pass.
- Observer and media-query tests pass.

### Regression

- None of the established 140 strict pixel passes regress.
- `clipping`, `water`, and `materials_basic` retain their established output.
- Multi-canvas no longer errors and all canvases are present with correct boxes.
- Multi-elements uses the shared canvas at the correct viewport size.
- Three repeated runs produce identical hashes for every deterministic example.
- Remaining failures are demonstrably outside the Blitz/LinkeDOM boundary.

### Cleanliness

- No example/addon/source-loader modification.
- No stubs, feature flags, fallbacks, test-name checks, or dead diagnostics.
- No fake CSS parser or hand-written layout approximation remains.
- Every local Blitz patch has a regression test and upstream-ready rationale.
- Documentation states state ownership, protocol version, flush lifecycle, alpha/color model, and resource readiness semantics.

## 17. Required deletion checklist

Before declaring completion, source searches must confirm removal of:

- `data-three-native-node`
- `document.documentElement.outerHTML` bridge transport
- temporary stylesheet link renaming
- bridge-created `<style>` used for base/layout corrections
- `canvas { display: inline-block }` correction
- `html, body { min-width/min-height: 100% }` correction
- current `globalThis.getComputedStyle` Proxy
- canvas-only `clientWidth/clientHeight` approximation
- direct external mutation of Blitz `SpecialElementData`
- `exact_fullscreen_canvas`
- visible-direct-text compositor heuristic
- legacy canvas ordering/counting compositor state
- temporary probe logs and bridge tree dumps

No deletion is deferred to a later cleanup session; cleanup is part of the atomic cutover.
