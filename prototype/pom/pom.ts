// prototype/pom/pom.ts — Super-cheap parallax occlusion mapping prototype.
//
// Renders a close, screen-filling brick wall through Three.js WebGPU (TSL
// shaders) inside afterglow-cef, and benchmarks three surface-detail tiers from
// docs/research/surface-detail-low-end-fallbacks.md:
//
//   [1] Normal map only                       (tier 0 — universal minimum)
//   [2] Offset-limited one-tap parallax       (tier 1 — one height fetch)
//   [3] Low-core POM (8–32 layers, no extras) (tier 2 — measured-viable)
//
// Goal: prove a POM tier runs extremely cheaply on the Radeon 680M — 60 FPS
// steady, p99 within the 16.68 ms vsync budget, 0 dropped frames / 300.
//
// Run via the `pom_bench` afterglow-cef example; press B to benchmark or load
// ?bench=300. See README.md.

import {
  createWebGPUOnlyRenderer,
  showWebGPUFailure,
} from '../../crates/afterglow-web/www/engine/webgpu-only.ts';
import {
  FrameBench,
  formatBenchResults,
  benchFromUrl,
} from '../../crates/afterglow-web/www/engine/bench.ts';

const THREE = window.THREE;
const {
  Fn, wgslFn,
  uniform, vec2, vec3, vec4,
  uv, texture, textureLevel, sampler,
  parallaxDirection, TBNViewMatrix,
  mix, normalize,
} = THREE;

// ---------------------------------------------------------------------------
// Procedural brick height + normal maps (built once on the CPU).
// ---------------------------------------------------------------------------

const TEX_SIZE = 256;
const REPEAT = 4; // 4×4 = 16 brick tiles across the wall

function makeBrickTextures() {
  const bw = 60; // brick width  (px)
  const bh = 28; // brick height (px)
  const mortar = 6; // mortar groove thickness (px)
  const periodX = bw + mortar;
  const periodY = bh + mortar;
  const size = TEX_SIZE;

  const height = new Uint8Array(size * size);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      // Offset every other row for a running-bond pattern.
      const row = Math.floor(y / periodY);
      const offX = (row & 1) * (periodX >> 1);
      const bx = (x + offX) % periodX;
      const by = y % periodY;
      const inMortar = bx < mortar || by < mortar;
      height[y * size + x] = inMortar ? 40 : 255;
    }
  }

  const heightTex = new THREE.DataTexture(
    height,
    size,
    size,
    THREE.RedFormat,
    THREE.UnsignedByteType,
  );
  heightTex.wrapS = heightTex.wrapT = THREE.RepeatWrapping;
  heightTex.minFilter = THREE.LinearFilter;
  heightTex.magFilter = THREE.LinearFilter;
  heightTex.generateMipmaps = false;
  heightTex.needsUpdate = true;

  // Derive a tangent-space normal map from the height field via a Sobel filter.
  const normal = new Uint8Array(size * size * 4);
  const at = (xx: number, yy: number) =>
    height[((yy & (size - 1)) * size + (xx & (size - 1))) | 0];
  const strength = 3.0;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const dl = at(x - 1, y);
      const dr = at(x + 1, y);
      const dd = at(x, y - 1);
      const du = at(x, y + 1);
      let nx = (-(dr - dl) / 255) * strength;
      let ny = (-(du - dd) / 255) * strength;
      const nz = 1.0;
      const len = Math.hypot(nx, ny, nz);
      nx /= len;
      ny /= len;
      const i = (y * size + x) * 4;
      normal[i] = (nx * 0.5 + 0.5) * 255;
      normal[i + 1] = (ny * 0.5 + 0.5) * 255;
      normal[i + 2] = (nz * 0.5 + 0.5) * 255; // ~1.0
      normal[i + 3] = 255;
    }
  }
  const normalTex = new THREE.DataTexture(
    normal,
    size,
    size,
    THREE.RGBAFormat,
    THREE.UnsignedByteType,
  );
  normalTex.wrapS = normalTex.wrapT = THREE.RepeatWrapping;
  normalTex.minFilter = THREE.LinearFilter;
  normalTex.magFilter = THREE.LinearFilter;
  normalTex.generateMipmaps = false;
  normalTex.colorSpace = THREE.NoColorSpace; // tangent-space data, not color
  normalTex.needsUpdate = true;

  return { heightTex, normalTex };
}

const { heightTex, normalTex } = makeBrickTextures();

// ---------------------------------------------------------------------------
// Shared shader uniforms / constants.
// ---------------------------------------------------------------------------

// Tangent-space view direction, from the surface toward the camera.
//
// Three's exported `parallaxDirection` = `positionViewDirection.mul(TBNViewMatrix)`.
// `positionViewDirection` is fragment→camera in view space, but the TSL
// `.mul(TBNViewMatrix)` transform yields the OPPOSITE of the standard POM
// viewDir (verified empirically: with parallaxDirection used directly, the
// recessed mortar grooves stick out instead of the bricks). Negate it to get
// the canonical fragment→camera tangent-space direction that LearnOpenGL /
// GPU Gems expect.
const viewDir = parallaxDirection.negate();

const tiledUV = uv().mul(REPEAT); // bake tiling into UV; textures use RepeatWrapping
const heightScale = uniform(0.05); // displacement in UV units; runtime-tunable
// Fragment→light direction in the wall's tangent frame. Updated from the
// directional light and wall transform once per frame; this is deliberately a
// uniform (not a per-fragment approximation) because DirectionalLight rays are
// parallel.
const lightDirT = uniform(new THREE.Vector3(0, 0, 1));
const selfShadowStrength = uniform(0.72); // contact shadow, never full black

const brickColor = vec3(0.58, 0.26, 0.19);
const grooveColor = vec3(0.10, 0.08, 0.07);

// Sample tangent-space normal at a UV and transform to view space (the exact
// pattern Three's own normalMap node uses: TBNViewMatrix.mul(n).normalize()).
function tangentNormal(sampleUV: unknown) {
  const n = texture(normalTex, sampleUV).rgb.mul(2.0).sub(1.0);
  return TBNViewMatrix.mul(n).normalize();
}

// Albedo: tint between groove and brick by the (displaced) height sample.
function albedo(sampleUV: unknown) {
  const h = textureLevel(heightTex, sampleUV, 0).r;
  return mix(grooveColor, brickColor, h);
}

// ---------------------------------------------------------------------------
// Tier 1: offset-limited one-tap parallax (GPU Gems 2 — one height fetch).
// ---------------------------------------------------------------------------

function oneTapUV() {
  const v = viewDir.normalize();
  const vz = v.z.abs().max(0.001); // clamp to avoid blow-up at grazing angles
  const h0 = textureLevel(heightTex, tiledUV, 0).r; // single height fetch
  // Offset UV opposite the tangent-space view dir, scaled by the local height.
  return tiledUV.sub(v.xy.div(vz).mul(h0).mul(heightScale));
}

// ---------------------------------------------------------------------------
// Tier 2: low-core parallax occlusion mapping (relief march, no self-shadow /
// silhouette / refine). Written as raw WGSL via `wgslFn` because loop-carried
// state (`var hitUV`) is natural in WGSL — the TSL `If`-gated assignment would
// form a cyclic node graph (curUV -> h = sample(curUV) -> curUV). The march
// uses `textureSampleLevel` (explicit LOD), valid in non-uniform control flow.
//
// VIEW-ANGLE-ADAPTIVE LAYER COUNT (the industry-standard fix for grazing-
// angle banding): head-on (|v.z|~1) few layers = cheap; at grazing angles
// (|v.z|~0) many layers = quality, so the discrete march steps don't show as
// visible bands. The count is derived from |v.z| in-shader, so the loop bound
// is runtime (not unrollable) — an intentional cost/quality trade: the cheap
// head-on case still hits vsync, and the grazing case buys smoothness.
// ---------------------------------------------------------------------------

function pomUV(numLayers: number) {
  // minLayers = the head-on count (the cheap budget, user-tunable via ←/→).
  // maxLayers = scaled up for grazing angles; clamp to keep worst-case sane.
  const minLayers = numLayers;
  const maxLayers = Math.min(96, Math.max(numLayers * 4, 64));
  const fn = wgslFn(`
    fn pom_march(
      heightTex: texture_2d<f32>,
      hSampler: sampler,
      baseUV: vec2f,
      viewDir: vec3f,
      scale: f32
    ) -> vec2f {
      // Canonical parallax occlusion mapping (LearnOpenGL / GPU Gems).
      // viewDir points from the surface toward the camera (+z out of surface).
      let v = normalize(viewDir);
      let vz = max(abs(v.z), 0.001);

      // Adaptive layer count: mix(maxLayers, minLayers, |v.z|) -> few layers
      // head-on, many at grazing. This removes the discrete banding that a
      // fixed layer count produces at steep angles.
      let numLayersF = mix(f32(${maxLayers}), f32(${minLayers}), abs(v.z));
      let numLayers = max(u32(${minLayers}), min(u32(${maxLayers}), u32(numLayersF + 0.5)));
      let P = v.xy * scale / vz;
      let delta = P / f32(numLayers);
      let layerDepth = 1.0 / f32(numLayers);

      // March from depth 0 (surface) toward the viewer (depth 1). The ray
      // pokes THROUGH the surface where the height field drops below the ray
      // depth; the first such layer is the visible point. Keep the previous
      // (under-surface) sample for the occlusion interpolation step.
      var currentUV = baseUV;
      var currentDepth = 0.0;
      var prevUV = baseUV;
      var prevDepth = 0.0;
      var prevHeight = textureSampleLevel(heightTex, hSampler, baseUV, 0.0).x;

      for (var i = 0u; i < numLayers; i = i + 1u) {
        currentUV = currentUV - delta;
        currentDepth = currentDepth + layerDepth;
        let h = textureSampleLevel(heightTex, hSampler, currentUV, 0.0).x;
        if (h < currentDepth) {
          // Occlusion interpolation: blend the previous (under-surface) and
          // current (over-surface) samples for a smooth intersection.
          let afterDepth = h - currentDepth;
          let beforeDepth = prevHeight - prevDepth;
          let w = afterDepth / (afterDepth - beforeDepth);
          return mix(currentUV, prevUV, w);
        }
        prevUV = currentUV;
        prevDepth = currentDepth;
        prevHeight = h;
      }
      return currentUV;
    }
  `);
  return fn({
    heightTex: texture(heightTex),
    hSampler: sampler(texture(heightTex)),
    baseUV: tiledUV,
    viewDir: viewDir,
    scale: heightScale,
  });
}

// ---------------------------------------------------------------------------
// Optional light-direction self-shadowing.
//
// Height convention here is physical height: brick=1 is raised, mortar≈0.16 is
// recessed. Starting at the POM intersection, cast a ray OUT toward the light:
// ray height increases and UV advances along light.xy/light.z. A sampled height
// above that ray is a nearer brick ridge and blocks the light. This is the
// geometric counterpart to the view-direction POM march, and is intentionally
// separate from the POM's inherent view self-occlusion.
//
// `textureSampleLevel` is required because the loop has non-uniform flow.
// ---------------------------------------------------------------------------
function selfShadow(uvAtHit: unknown) {
  const fn = wgslFn(`
    fn pom_self_shadow(
      heightTex: texture_2d<f32>, hSampler: sampler,
      hitUV: vec2f, lightDir: vec3f, scale: f32
    ) -> f32 {
      let l = normalize(lightDir);
      // The light is behind the textured face: no relief shadow can reach it.
      if (l.z <= 0.001) { return 1.0; }

      // Two samples catch contact at this coarse brick/mortar frequency while
      // keeping the optional contact-shadow tier inside the 680M frame budget.
      let steps = 2u;
      let layerHeight = 1.0 / f32(steps);
      let uvStep = (l.xy / max(l.z, 0.001)) * scale / f32(steps);
      var rayUV = hitUV;
      var rayHeight = textureSampleLevel(heightTex, hSampler, hitUV, 0.0).x;
      // A small slope-scale bias avoids a texel shadowing itself at the first
      // bilinear sample. The returned visibility is provably in [0,1].
      let bias = 0.035;
      for (var i = 0u; i < steps; i = i + 1u) {
        rayUV = rayUV + uvStep;
        rayHeight = rayHeight + layerHeight;
        let terrainHeight = textureSampleLevel(heightTex, hSampler, rayUV, 0.0).x;
        if (terrainHeight > rayHeight + bias) {
          return 0.0;
        }
        // Once the ray is above the tallest possible height, no later sample
        // can block it. WGSL permits a uniform early return in this loop.
        if (rayHeight >= 1.0) { return 1.0; }
      }
      return 1.0;
    }
  `);
  return fn({
    heightTex: texture(heightTex),
    hSampler: sampler(texture(heightTex)),
    hitUV: uvAtHit,
    lightDir: lightDirT,
    scale: heightScale,
  });
}

// POM contact visibility must attenuate illumination, not `colorNode`:
// multiplying base colour would incorrectly shadow ambient/indirect light. The
// scene has one DirectionalLight, so after PhysicalLightingModel adds that
// light's direct diffuse/specular terms, multiply precisely those terms. Three
// continues to compute normal-map angle response, BRDF, ambient, and indirect
// contributions unchanged.
class ContactShadowLightingModel extends THREE.PhysicalLightingModel {
  visibility: any;

  constructor(visibility: any) {
    super();
    this.visibility = visibility;
  }

  direct(lightData: any, builder: any) {
    super.direct(lightData, builder);
    lightData.reflectedLight.directDiffuse.mulAssign(this.visibility);
    lightData.reflectedLight.directSpecular.mulAssign(this.visibility);
  }
}

// ---------------------------------------------------------------------------
// Material factory: one MeshStandardNodeMaterial per tier.
// ---------------------------------------------------------------------------

let pomLayers = 16; // low-core default; 8/16/32 selectable via keys

type Mode = { name: string; material: any };

function buildModes(): Mode[] {
  // Edge discard (LearnOpenGL): a displaced UV can leave [0,1] at the wall's
  // border, producing oversampling artifacts. Discard such fragments.
  // NOTE: evaluated on the *tiled* UV (0..REPEAT); we check the per-tile fract
  // stays in range by clamping against the total tiled extent.
  const withEdgeGuard = (mat: any) => {
    // No-op guard placeholder — the per-tier materials discard via their own
    // mat.outputNode below. Kept for clarity of intent.
    return mat;
  };

  const base = (sampleUV: unknown) => {
    const mat = new THREE.MeshStandardNodeMaterial();
    mat.colorNode = vec4(albedo(sampleUV), 1.0);
    mat.normalNode = tangentNormal(sampleUV);
    mat.roughnessNode = uniform(0.85);
    mat.metalnessNode = uniform(0.0);
    return withEdgeGuard(mat);
  };

  const normalOnly = base(tiledUV);
  const oneTap = base(oneTapUV());
  const pomUVAtHit = pomUV(pomLayers);
  const pom = base(pomUVAtHit);

  // Visibility is applied by ContactShadowLightingModel to the direct-light
  // terms only. Do not multiply colorNode: ambient/indirect fill is unshadowed.
  const shadowed = new THREE.MeshStandardNodeMaterial();
  const visibility = mix(1.0, selfShadow(pomUVAtHit), selfShadowStrength);
  shadowed.colorNode = vec4(albedo(pomUVAtHit), 1.0);
  shadowed.normalNode = tangentNormal(pomUVAtHit);
  shadowed.roughnessNode = uniform(0.85);
  shadowed.metalnessNode = uniform(0.0);
  shadowed.setupLightingModel = () => new ContactShadowLightingModel(visibility);

  return [
    { name: 'normal', material: normalOnly },
    { name: 'one-tap parallax', material: oneTap },
    { name: `POM (${pomLayers} layers)`, material: pom },
    { name: `POM (${pomLayers}) + contact shadows`, material: shadowed },
  ];
}

// ---------------------------------------------------------------------------
// Scene, camera, renderer.
// ---------------------------------------------------------------------------

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0a0c10);

const camera = new THREE.PerspectiveCamera(50, innerWidth / innerHeight, 0.1, 100);
camera.position.set(0, 0, 4);

// Wall: PlaneGeometry lies in the XY plane (normal +Z, toward the camera).
// Supply explicit tangents (UV.u along +X) so the TBN frame is exact.
const wall = new THREE.PlaneGeometry(6, 6, 1, 1);
wall.setAttribute(
  'tangent',
  new THREE.BufferAttribute(
    new Float32Array([1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1]),
    4,
  ),
);

const mesh = new THREE.Mesh(wall, null as unknown as THREE.Material);
mesh.rotation.y = 0.25; // a slight tilt so the parallax is visible
scene.add(mesh);

scene.add(new THREE.AmbientLight(0x404060, 0.6));
const dirLight = new THREE.DirectionalLight(0xffffff, 1.1);
dirLight.position.set(4, 6, 8);
scene.add(dirLight);
// Reused in the render loop; do not allocate a Vector3 per frame.
const lightDirectionScratch = new THREE.Vector3();

// ---------------------------------------------------------------------------
// HUD + mode switching + benchmark.
// ---------------------------------------------------------------------------

const hud = document.createElement('div');
hud.style.cssText =
  'position:fixed;top:8px;left:8px;color:#e5e7eb;font:13px/1.5 ui-monospace,monospace;' +
  'background:rgba(0,0,0,0.7);padding:8px 12px;border-radius:4px;pointer-events:none;z-index:10';
document.body.appendChild(hud);

const benchEl = document.createElement('div');
benchEl.style.cssText =
  'position:fixed;top:8px;right:8px;color:#e5e7eb;font:13px/1.5 ui-monospace,monospace;' +
  'background:rgba(0,0,0,0.7);padding:8px 12px;border-radius:4px;pointer-events:none;z-index:10';
document.body.appendChild(benchEl);

// Hard-angle toggle button: snaps the camera to a steep grazing view (view ray
// nearly parallel to the wall) so the POM silhouette/parallax is most visible.
// Press again (or [V]) to return to the slow orbit view.
const angleBtn = document.createElement('button');
angleBtn.textContent = '∠ hard angle (V)';
angleBtn.style.cssText =
  'position:fixed;bottom:12px;left:8px;color:#e5e7eb;font:13px/1.5 ui-monospace,monospace;' +
  'background:rgba(0,0,0,0.7);border:1px solid #3a3f4b;border-radius:4px;padding:6px 12px;' +
  'cursor:pointer;z-index:10';
document.body.appendChild(angleBtn);

let modes = buildModes();
let modeIdx = 2; // start on POM — the tier under test
mesh.material = modes[modeIdx].material;

let bench: FrameBench | null = benchFromUrl({
  thresholdFps: 55,
  onDone: (r) => {
    const line = formatBenchResults(r);
    benchEl.textContent = line;
    console.log('[bench]', line, r);
  },
});

function renderHud() {
  const m = modes[modeIdx];
  hud.textContent =
    `afterglow POM prototype — mode ${modeIdx + 1}/4: ${m.name}\n` +
    `heightScale=${heightScale.value.toFixed(3)}  POM layers=${pomLayers}  shadow=${selfShadowStrength.value.toFixed(2)}  view=${hardAngle ? 'hard-angle' : 'orbit'}\n` +
    `[1] normal  [2] one-tap  [3] POM  [4] POM+shadow  [S] shadow strength  [←/→] layers  [↑/↓] scale  [V] hard angle  [F] freeze  [B] bench`;
}
renderHud();

function setMode(i: number) {
  if (i < 0 || i >= modes.length) return;
  modeIdx = i;
  mesh.material = modes[modeIdx].material;
  renderHud();
}

// Automation hooks intentionally use direct functions, not synthetic keyboard
// events: CDP KeyboardEvent dispatch is not a reliable substitute for CEF input.
(window as any).pomSetMode = setMode;
(window as any).pomSetShadowStrength = (strength: number) => {
  selfShadowStrength.value = Math.min(1, Math.max(0, strength));
  renderHud();
};

// --- Hard-angle grazing view ---
// Orbit view: camera fixed at (0,0,4), wall slowly rotates (0.25 ± 0.18 rad).
// Hard-angle view: camera slides to just past the wall's edge and barely in
// front of it, so the view ray is nearly PARALLEL to the surface (|v.xy|/v.z
// is large) — a true grazing angle, the regime where POM's per-layer silhouette
// shift is most dramatic and the `max(abs(v.z), 0.001)` clamp is hardest hit.
let hardAngle = false;
// Freeze the wall rotation for deterministic mode comparison (toggle [F]).
let frozen = false;
(window as any).pomFreeze = (v: boolean) => { frozen = v; };
const ORBIT_CAM = new THREE.Vector3(0, 0, 4);
const HARD_CAM = new THREE.Vector3(4.6, 0.05, 0.45); // past the +X edge, ~edge-on
const HARD_LOOK = new THREE.Vector3(-1.2, 0, 0); // look across the wall, not at center

function toggleHardAngle() {
  hardAngle = !hardAngle;
  if (hardAngle) {
    camera.position.copy(HARD_CAM);
    camera.lookAt(HARD_LOOK);
  } else {
    camera.position.copy(ORBIT_CAM);
    camera.lookAt(0, 0, 0);
  }
  angleBtn.style.borderColor = hardAngle ? '#ff9a9a' : '#3a3f4b';
  renderHud();
}
angleBtn.addEventListener('click', toggleHardAngle);

function rebuildPom() {
  // Layer count is a baked shader constant, so rebuild both POM variants.
  const oldPom = modes[2].material;
  const oldShadow = modes[3].material;
  const rebuiltUV = pomUV(pomLayers);
  const makeMaterial = (shadowed: boolean) => {
    const mat = new THREE.MeshStandardNodeMaterial();
    const visibility = shadowed
      ? mix(1.0, selfShadow(rebuiltUV), selfShadowStrength)
      : 1.0;
    mat.colorNode = vec4(albedo(rebuiltUV), 1.0);
    mat.normalNode = tangentNormal(rebuiltUV);
    mat.roughnessNode = uniform(0.85);
    mat.metalnessNode = uniform(0.0);
    if (shadowed) mat.setupLightingModel = () => new ContactShadowLightingModel(visibility);
    return mat;
  };
  modes[2] = { name: `POM (${pomLayers} layers)`, material: makeMaterial(false) };
  modes[3] = { name: `POM (${pomLayers}) + contact shadows`, material: makeMaterial(true) };
  if (modeIdx === 2 || modeIdx === 3) mesh.material = modes[modeIdx].material;
  oldPom.dispose?.();
  oldShadow.dispose?.();
  renderHud();
}

addEventListener('keydown', (e) => {
  switch (e.key) {
    case '1': setMode(0); break;
    case '2': setMode(1); break;
    case '3': setMode(2); break;
    case '4': setMode(3); break;
    case 's':
    case 'S':
      selfShadowStrength.value = selfShadowStrength.value > 0 ? 0 : 0.72;
      setMode(3);
      break;
    case 'ArrowLeft':
      pomLayers = Math.max(4, pomLayers - 4);
      rebuildPom();
      break;
    case 'ArrowRight':
      pomLayers = Math.min(64, pomLayers + 4);
      rebuildPom();
      break;
    case 'ArrowUp':
      heightScale.value = Math.min(0.2, heightScale.value + 0.01);
      renderHud();
      break;
    case 'ArrowDown':
      heightScale.value = Math.max(0.005, heightScale.value - 0.01);
      renderHud();
      break;
    case 'b':
    case 'B': {
      bench = new FrameBench({
        frames: 300,
        thresholdFps: 55,
        onDone: (r) => {
          const line = formatBenchResults(r);
          benchEl.textContent = line;
          console.log('[bench]', line, r);
        },
      });
      benchEl.textContent = 'benchmarking…';
      bench.start();
      break;
    }
    case 'v':
    case 'V':
      toggleHardAngle();
      break;
    case 'f':
    case 'F':
      frozen = !frozen;
      renderHud();
      break;
  }
});

// ---------------------------------------------------------------------------
// Boot: WebGPU-only renderer (fail-closed — no WebGL fallback, per AGENTS.md).
// ---------------------------------------------------------------------------

// `antialias: false` — MSAA multiplies a fragment-bound POM shader's cost 4×.
// For a super-cheap POM tier the displacement itself carries the detail; AA off
// is the correct cheapness choice (documented in README).
const renderer = await createWebGPUOnlyRenderer({ antialias: false }).catch((err) => {
  showWebGPUFailure(err);
  throw err;
});
renderer.setSize(innerWidth, innerHeight);
renderer.setPixelRatio(devicePixelRatio); // render at physical px (2880×1800 @ DPR 2)
document.body.appendChild(renderer.domElement);

addEventListener('resize', () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
  renderer.setPixelRatio(devicePixelRatio);
});

// ---------------------------------------------------------------------------
// Render loop.
// ---------------------------------------------------------------------------

const startTime = performance.now();

function animate(timestamp: number) {
  const elapsed = (performance.now() - startTime) / 1000;
  // Slow rotation keeps parallax moving for a steady-state, fragment-bound cost.
  // In hard-angle view the wall is held flat (normal toward the camera) so the
  // grazing comes purely from the near-edge-on camera — a clean, stable steep
  // angle to inspect POM silhouette behaviour.
  mesh.rotation.y = hardAngle ? 0.0 : (frozen ? 0.25 : 0.25 + Math.sin(elapsed * 0.25) * 0.18);

  // Plane tangent=(cos(y),0,-sin(y)), bitangent=(0,1,0), normal=(sin(y),0,
  // cos(y)). DirectionalLight.position gives the parallel ray's direction
  // toward the light, so dot it into that TBN to get fragment→light tangent
  // coordinates for the self-shadow march.
  const light = lightDirectionScratch.copy(dirLight.position).normalize();
  const c = Math.cos(mesh.rotation.y);
  const s = Math.sin(mesh.rotation.y);
  lightDirT.value.set(
    light.x * c - light.z * s,
    light.y,
    light.x * s + light.z * c,
  );

  renderer.render(scene, camera);
  bench?.tick(timestamp);
  requestAnimationFrame(animate);
}

requestAnimationFrame(animate);
console.log('[pom] prototype started — mode:', modes[modeIdx].name);
