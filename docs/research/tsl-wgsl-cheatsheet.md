# Three.js WebGPU TSL/WGSL Shader Cheatsheet

Research date: 2026-07-12
Three.js version: 0.185+

## Overview

Three.js WebGPU uses TSL (Three.js Shading Language) as the shader authoring
API. TSL is a JavaScript-native node system that compiles to WGSL (WebGPU)
or GLSL (WebGL). You can write shaders in three styles:

1. **Functional TSL** — JS-like syntax with chained method calls
2. **Raw WGSL** — via `wgslFn()` (pure WGSL strings)
3. **Raw GLSL** — via `glslFn()` (WebGL only, for legacy)

For complex shaders (like virtual texturing), `wgslFn()` is recommended —
the shader logic is pure WGSL, TSL just handles binding plumbing.

---

## Setup

```js
import * as THREE from 'three/webgpu';
import { 
  Fn, wgslFn, glslFn, code,
  uniform, texture, sampler, storageTexture, storage, instancedArray,
  uv, positionLocal, positionWorld, normalLocal, normalWorld,
  cameraPosition, modelViewMatrix, modelWorldMatrix,
  time, deltaTime, frameId,
  float, vec2, vec3, vec4, mat3, mat4, int, uint, bool,
  If, Switch, Loop, Break, Continue, Discard, Return,
  varying, element, instanceIndex,
} from 'three/tsl';

// Renderer
const renderer = new THREE.WebGPURenderer({ antialias: true });
await renderer.init();
```

---

## Materials

Every stock material has a `NodeMaterial` equivalent:

| Classic Material | Node Material |
|-----------------|---------------|
| `MeshStandardMaterial` | `MeshStandardNodeMaterial` |
| `MeshBasicMaterial` | `MeshBasicNodeMaterial` |
| `MeshPhongMaterial` | `MeshPhongNodeMaterial` |
| `MeshPhysicalMaterial` | `MeshPhysicalNodeMaterial` |
| `MeshNormalMaterial` | `MeshNormalNodeMaterial` |
| — | `NodeMaterial` (from scratch) |

### Node hooks (override points)

```js
const material = new THREE.MeshStandardNodeMaterial();

material.colorNode    = ...; // vec4 — fragment color
material.positionNode = ...; // vec3 — vertex position (local space)
material.normalNode   = ...; // vec3 — vertex normal (view space)
material.depthNode    = ...; // float — depth
material.alphaNode    = ...; // float — alpha
material.emissiveNode = ...; // vec3 — emissive
material.metalnessNode = ...; // float
material.roughnessNode = ...; // float
material.outputNode   = ...; // vec4 — final output
```

---

## TSL Syntax

### Types

```js
float(1.0)           // f32
int(1)                // i32
uint(1)               // u32
bool(true)            // bool
vec2(0.0, 1.0)        // vec2<f32>
vec3(0, 0, 0)         // vec3<f32>
vec4(1, 0, 0, 1)      // vec4<f32>
uvec2(0, 0)           // vec2<u32>
ivec3(0, 0, 0)        // vec3<i32>
mat3(...)              // mat3x3<f32>
mat4(...)              // mat4x4<f32>
```

### Operators (chained method calls)

```js
a.add(b)         // a + b
a.sub(b)         // a - b
a.mul(b)         // a * b
a.div(b)         // a / b
a.mod(b)         // a % b
a.negate()       // -a
a.dot(b)         // dot(a, b)
a.cross(b)       // cross(a, b)
a.normalize()    // normalize(a)
a.length()       // length(a)
a.abs()          // abs(a)
a.floor()        // floor(a)
a.fract()        // fract(a)
a.pow(b)         // pow(a, b)
a.min(b)         // min(a, b)
a.max(b)         // max(a, b)
a.clamp(min, max) // clamp(a, min, max)
a.mix(b, t)      // mix(a, b, t)
a.smoothstep(b, c) // smoothstep(b, c, a)
a.lessThan(b)    // a < b (returns bool node)
a.greaterThan(b) // a > b
a.equal(b)       // a == b

// Swizzles
vec4(...).rgb    // .xyz → vec3
vec4(...).xy     // .xy → vec2
vec3(...).xz     // .xz → vec2

// Component access
vec3(...).x      // float
vec4(...).w      // float
```

### Math functions

```js
import { abs, floor, ceil, fract, sin, cos, tan, pow, exp, exp2, log, log2,
         sqrt, min, max, clamp, mix, smoothstep, step, length, normalize,
         dot, cross, reflect, refract, distance, faceForward } from 'three/tsl';
```

---

## Writing Shaders with Fn()

`Fn()` creates a reusable TSL function. Call with `()` to instantiate.

### Fragment color example

```js
const colorNode = Fn(() => {
  const uvCoord = uv();
  const red = uvCoord.x.add(2.3).mul(0.3);
  const green = uvCoord.y.add(1.7).div(8.2);
  const blue = uvCoord.x.add(uvCoord.y).mod(10.0);
  return vec4(red, green, blue, 1.0);
})();

const material = new THREE.MeshBasicNodeMaterial();
material.colorNode = colorNode;
```

### With arguments

```js
// Array-style args
const myFn = Fn(([pos, time]) => {
  return pos.add(vec3(time, 0, 0));
});

// Object-style args (named)
const myFn = Fn(({ position, time }) => {
  return position.add(vec3(time, 0, 0));
});
```

### Control flow

```js
// If/Else
If(condition, () => {
  // then branch
}).elseif(condition2, () => {
  // elseif branch
}).else(() => {
  // else branch
});

// Select (ternary)
const result = condition.select(trueValue, falseValue);

// Loop
Loop(10, ({ i }) => {
  // i = 0..9
});

Loop({ start: 0, end: 10, type: 'int', condition: '<' }, ({ i }) => {
  // ...
});

// Nested
Loop(10, 5, ({ i, j }) => {
  // i = 0..9, j = 0..4
});

// While
Loop(condition.lessThan(10), () => {
  // ...
});

// Control
Break();
Continue();
Discard();       // discard fragment
Return();        // early return
```

### Variables

```js
// toVar() — create a mutable variable
const color = vec3(1, 0, 0).toVar();
color.assign(vec3(0, 1, 0)); // mutate
```

### Varyings

```js
// Declare a varying (vertex → fragment)
const vNormal = varying(vec3(), 'vNormal');

// In positionNode:
vNormal.assign(computedNormal);

// In normalNode:
return vNormal;
```

---

## Raw WGSL with wgslFn()

Write pure WGSL. TSL handles bind group plumbing.

### Basic WGSL function

```js
const myShader = wgslFn(`
  fn myShader(
    tex: texture_2d<f32>,
    sampler: sampler,
    uv: vec2f,
    color: vec4f
  ) -> vec4f {
    let sampled = textureSample(tex, sampler, uv);
    return mix(sampled, color, 0.5);
  }
`);

// Call from TSL
const result = myShader({
  tex: texture(myTexture),
  sampler: sampler(texture(myTexture)),
  uv: uv(),
  color: vec4(1, 0, 0, 1),
});
```

### Reusable WGSL snippets (includes)

```js
// Helper functions
const helpers = code(`
  fn hash(p: vec2f) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
  }
  
  fn noise(p: vec2f) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let a = hash(i);
    let b = hash(i + vec2(1.0, 0.0));
    let c = hash(i + vec2(0.0, 1.0));
    let d = hash(i + vec2(1.0, 1.0));
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
  }
`);

// Main function includes helpers
const mainFn = wgslFn(`
  fn main(uv: vec2f, time: f32) -> vec4f {
    let n = noise(uv * 10.0 + time);
    return vec4f(n, n, n, 1.0);
  }
`, [helpers]); // ← pass includes array
```

### WGSL with storage buffers

```js
const myComputeFn = wgslFn(`
  fn compute(
    buffer: ptr<storage, array<vec3f>, read_write>,
    count: u32,
    index: u32
  ) -> void {
    if (index >= count) { return; }
    buffer[index] = vec3f(f32(index), 0.0, 0.0);
  }
`);
```

---

## Uniforms

```js
// Scalar
const time = uniform(0.0);
time.value = clock.getElapsedTime(); // update each frame

// Vector
const offset = uniform(new THREE.Vector2(0, 0));

// Color
const baseColor = uniform(new THREE.Color(1, 0, 0));

// Matrix
const transform = uniform(new THREE.Matrix4());

// Access in shader
const x = time.value; // JS side
const x = time;        // shader side (TSL node)
```

### Built-in uniforms

```js
import { time, deltaTime, frameId,
         cameraPosition, cameraNear, cameraFar,
         modelWorldMatrix, modelViewMatrix } from 'three/tsl';

// In shader:
time        // f32 — elapsed time (seconds)
deltaTime   // f32 — frame delta time
cameraPosition // vec3 — camera world position
```

---

## Textures

### Sample a texture

```js
// Basic sampling
const color = texture(myTexture, uv());

// With explicit UV
const color = texture(myTexture, uv().mul(2.0));

// With mip level (textureLoad — no sampler, no filtering)
const color = textureLoad(myTexture, uv(), 0);

// With bias
const color = texture(myTexture, uv()).bias(1.0);

// With explicit level
const color = textureLevel(myTexture, uv(), 2);
```

### Texture as uniform (for wgslFn)

```js
// For wgslFn, wrap texture + sampler
const texNode = texture(myTexture);
const samplerNode = sampler(texNode);

const result = myShader({
  tex: texNode,
  sampler: samplerNode,
  uv: uv(),
});
```

### Integer textures (for page tables)

```js
// Create an integer texture for page table
const pageTable = new THREE.DataTexture(
  new Uint8Array(...), size, size,
  THREE.RGBAIntegerFormat, THREE.UnsignedByteType
);
pageTable.minFilter = THREE.NearestFilter;
pageTable.magFilter = THREE.NearestFilter;
pageTable.generateMipmaps = false;

// In WGSL, declare as texture_2d<u32>
const shader = wgslFn(`
  fn lookup(pageTable: texture_2d<u32>, uv: vec2f) -> vec4u {
    return textureLoad(pageTable, vec2i(uv * 1024.0), 0);
  }
`);
```

---

## Storage Buffers & Compute Shaders

### Storage buffers

```js
// Create a storage buffer
const count = 1024;
const buffer = instancedArray(count, 'vec3'); // storage buffer

// Or from existing buffer
const storageNode = storage(myBufferAttribute, 'vec3', count);

// Access element by index
const element = buffer.element(instanceIndex);
```

### Compute shader

```js
const computeFn = Fn(() => {
  const pos = buffer.element(instanceIndex);
  pos.x = float(instanceIndex).mod(64.0);
  pos.z = float(instanceIndex).div(64.0);
})();

const computeNode = computeFn.compute(count); // dispatch count threads

// Execute
renderer.computeAsync(computeNode);
```

### WGSL compute shader

```js
const computeFn = wgslFn(`
  fn compute(
    buffer: ptr<storage, array<vec3f>, read_write>,
    count: u32,
    index: u32
  ) -> void {
    if (index >= count) { return; }
    let x = index % 64u;
    let z = index / 64u;
    buffer[index] = vec3f(f32(x), 0.0, f32(z));
  }
`);

const computeNode = computeFn({
  buffer: buffer,
  count: count,
  index: instanceIndex,
}).compute(count);
```

### Workgroup size

```js
// Default: [64, 1, 1]
computeNode.compute(count, [64, 1, 1]); // 64 threads per workgroup

// For 2D data (textures):
computeNode.compute(width * height, [8, 8, 1]); // 64 threads, 2D
```

### Storage textures (write from compute)

```js
const storageTex = new THREE.StorageTexture(width, height);

// Write to storage texture in compute shader
const writeFn = Fn(({ storageTexture }) => {
  const x = instanceIndex.mod(width);
  const y = instanceIndex.div(width);
  textureStore(storageTexture, uvec2(x, y), vec4(1, 0, 0, 1)).toWriteOnly();
});

const computeNode = writeFn({ storageTexture }).compute(width * height);
```

### GPU → CPU readback

```js
// Compute then read back
await renderer.computeAsync(computeNode);

// Read storage buffer back to CPU
const data = new Float32Array(
  await renderer.getArrayBufferAsync(storageBufferAttribute)
);
```

---

## Accessors (built-in nodes)

### Position
```js
positionGeometry  // vec3 — raw position attribute
positionLocal     // vec3 — local space (after skinning/instancing)
positionWorld     // vec3 — world space
positionView      // vec3 — view space
positionViewDirection // vec3 — view direction
```

### Normal
```js
normalGeometry    // vec3 — raw normal attribute
normalLocal       // vec3 — local space
normalWorld       // vec3 — world space
normalView        // vec3 — view space
transformNormalToView(normalNode) // convert local → view
```

### UV
```js
uv()              // vec2 — UV channel 0
uv(1)             // vec2 — UV channel 1
```

### Camera
```js
cameraPosition    // vec3 — camera world position
cameraNear        // f32
cameraFar         // f32
cameraViewMatrix  // mat4
cameraProjectionMatrix // mat4
cameraWorldMatrix // mat4
```

### Model (per-object)
```js
modelWorldMatrix     // mat4
modelViewMatrix      // mat4
modelNormalMatrix    // mat3
modelWorldMatrixInverse // mat4
modelScale           // vec3
```

### Other
```js
instanceIndex    // u32 — instance ID (compute or instanced)
vertexIndex      // u32 — vertex ID (compute)
frontFacing      // bool — is front face
screenUV         // vec2 — screen UV (0-1)
viewportUV       // vec2 — viewport UV
```

---

## Render Targets & Post-Processing

### Render to texture

```js
// Create render target
const rt = new THREE.RenderTarget(width, height, {
  format: THREE.RGBAFormat,
  type: THREE.UnsignedByteType,
  minFilter: THREE.NearestFilter,
  magFilter: THREE.NearestFilter,
});

// Render scene to RT
renderer.setRenderTarget(rt);
renderer.render(scene, camera);
renderer.setRenderTarget(null);

// Use RT texture in shader
const tex = texture(rt.texture, uv());
```

### Read render target pixels (GPU → CPU)

```js
// WebGPU: readback
const buffer = renderer.getRenderTargetBuffer(rt, 0);
const data = await renderer.getArrayBufferAsync(buffer);
const pixels = new Uint8Array(data);
```

---

## Virtual Texturing Pattern (for our engine)

### Page table lookup shader (WGSL)

```js
const vtSample = wgslFn(`
  fn vtSample(
    pageTable: texture_2d<u32>,
    atlas: texture_2d<f32>,
    atlasSampler: sampler,
    uv: vec2f,
    virtualSize: vec2f,
    pageGrid: vec2f,
    pageSize: f32,
    pageBorder: f32,
    atlasSize: vec2f
  ) -> vec4f {
    // Compute mip level from screen-space derivatives
    let effective_size = virtualSize;
    let dx = dpdx(uv * effective_size);
    let dy = dpdy(uv * effective_size);
    let texel_footprint = max(dot(dx, dx), dot(dy, dy));
    let mip_float = 0.5 * log2(max(texel_footprint, 1e-8));
    var mip_level = u32(clamp(mip_float, 0.0, 4.0));
    
    // Walk from desired mip up, looking for resident page
    var is_resident = false;
    var entry = 0u;
    var curr_page_grid = vec2f(0.0);
    
    for (var m = mip_level; m <= 4u; m = m + 1u) {
      let mip_scale = exp2(-f32(m));
      curr_page_grid = max(pageGrid * mip_scale, vec2f(1.0));
      let page_coords = vec2i(floor(uv * curr_page_grid));
      entry = textureLoad(pageTable, page_coords, i32(m)).r;
      if ((entry & 1u) != 0u) {
        is_resident = true;
        mip_level = m;
        break;
      }
    }
    
    if (!is_resident) {
      return vec4f(0.5, 0.5, 0.5, 1.0); // fallback gray
    }
    
    // Compute physical atlas UV
    let physX = (entry >> 1) & 0xFFu;
    let physY = (entry >> 9) & 0xFFu;
    let local_uv = fract(uv * curr_page_grid);
    let page_origin = vec2f(physX, physY) * (pageSize + pageBorder * 2.0);
    let half_padding = pageBorder;
    let sample_texel = page_origin + half_padding + local_uv * pageSize;
    let atlas_uv = sample_texel / atlasSize;
    
    return textureSample(atlas, atlasSampler, atlas_uv);
  }
`);
```

### Feedback shader (WGSL)

```js
const feedbackFn = wgslFn(`
  fn feedback(
    virtualSize: vec2f,
    pageGrid: vec2f,
    maxMip: f32,
    uv: vec2f
  ) -> u32 {
    let dx = dpdx(uv * virtualSize);
    let dy = dpdy(uv * virtualSize);
    let texel_footprint = max(dot(dx, dx), dot(dy, dy));
    let mip_float = clamp(0.5 * log2(max(texel_footprint, 1e-8)), 0.0, maxMip);
    let mip_level = u32(mip_float);
    let mip_scale = exp2(-f32(mip_level));
    let curr_page_grid = max(pageGrid * mip_scale, vec2f(1.0));
    let page_coords = floor(uv * curr_page_grid);
    
    // Pack: bit 31 = valid, bits 0-4 = mip, bits 5-12 = pageX, bits 13-20 = pageY
    return 0x80000000u | 
           (mip_level & 0x1Fu) |
           ((u32(page_coords.x) & 0xFFu) << 5) |
           ((u32(page_coords.y) & 0xFFu) << 13);
  }
`);
```

### Integrating VT into MeshStandardNodeMaterial

```js
const material = new THREE.MeshStandardNodeMaterial();

// Override the diffuse/color sampling with VT
material.colorNode = Fn(() => {
  return vtSample({
    pageTable: texture(pageTableTexture),
    atlas: texture(atlasTexture),
    atlasSampler: sampler(texture(atlasTexture)),
    uv: uv(),
    virtualSize: uniform(new THREE.Vector2(16384, 16384)),
    pageGrid: uniform(new THREE.Vector2(128, 128)),
    pageSize: uniform(128.0),
    pageBorder: uniform(4.0),
    atlasSize: uniform(new THREE.Vector2(2048, 2048)),
  });
})();
```

---

## Quick Reference Table

| What | TSL Function | WGSL Equivalent |
|------|-------------|-----------------|
| Declare uniform | `uniform(value)` | `uniform var` |
| Sample texture | `texture(tex, uv)` | `textureSample(tex, sampler, uv)` |
| Load texture (no filter) | `textureLoad(tex, uv, level)` | `textureLoad(tex, coords, level)` |
| Storage buffer | `storage(attr, 'vec3', count)` | `var<storage> buffer` |
| Storage texture | `storageTexture(tex)` | `var<storage, ...>` |
| Write to storage tex | `textureStore(tex, uv, val)` | `textureStore(tex, coords, val)` |
| Compute dispatch | `fn().compute(count)` | `@compute @workgroup_size(64)` |
| Instance index | `instanceIndex` | `instance_index` |
| Vertex index | `vertexIndex` | `vertex_index` |
| Screen derivatives | `dpdx(val)`, `dpdy(val)` | `dpdx(val)`, `dpdy(val)` |
| If/Else | `If(cond, () => {...}).else(() => {...})` | `if cond { ... } else { ... }` |
| Loop | `Loop(count, ({i}) => {...})` | `for (var i = 0; i < count; i++)` |
| Select (ternary) | `cond.select(a, b)` | `select(b, a, cond)` |
| Mutable var | `val.toVar()` | `var x = val;` |
| Varying | `varying(val, 'name')` | `@vertex fn ... -> ...` |
| Discard | `Discard()` | `discard;` |
| Early return | `Return()` | `return;` |

---

## Key Gotchas

1. **Texture as uniform**: `uniform()` doesn't support textures. Use `texture(tex)` instead.
2. **Sampler in WGSL**: WGSL needs explicit `sampler` argument. Use `sampler(texture(tex))`.
3. **wgslFn args**: Always pass as object: `fn({ name: value })`.
4. **Fn() instantiation**: Call with `()` to get a node: `Fn(() => {...})()`.
5. **Compute workgroup size**: Max 256 threads. Default `[64, 1, 1]`.
6. **Storage buffers**: Use `instancedArray()` or `storage()` for compute. `buffer()` for read-only.
7. **GPU→CPU latency**: `getArrayBufferAsync()` is async — 1-2 frame latency for readback.
8. **Integer textures**: For page tables, use `RGBAIntegerFormat` + `UnsignedByteType`.
9. **WebGL fallback**: `wgslFn` only works on WebGPU. `glslFn` only works on WebGL. `Fn()` works on both.
10. **dpdx/dpdy**: Screen-space derivatives. In TSL: `dpdx(val)`, `dpdy(val)`. In WGSL: `dpdx(val)`, `dpdy(val)`.
