// Bounded low-core parallax occlusion mapping for resident height textures.

/**
 * Adaptive 8–32-style POM march with interpolation and distance fade.
 * No silhouette, self-shadow, or secondary relief pass.
 */
export const POM_UV_WGSL = /* wgsl */ `
fn pomMarchUV(
  heightTexture: texture_2d<f32>, heightSampler: sampler,
  baseUV: vec2f, viewDir: vec3f, heightScale: f32, maxOffsetRatio: f32,
  minLayers: u32, maxLayers: u32, maxDistance: f32, viewDistance: f32
) -> vec2f {
  if (heightScale <= 0.0) {
    return baseUV;
  }
  var fade = 1.0;
  if (maxDistance > 0.0) {
    if (viewDistance >= maxDistance) { return baseUV; }
    fade = 1.0 - smoothstep(maxDistance * 0.65, maxDistance, viewDistance);
  }
  let scale = heightScale * fade;
  if (scale <= 0.00001) { return baseUV; }

  let v = normalize(viewDir);
  let vz = max(abs(v.z), 0.001);
  let low = max(1u, min(minLayers, maxLayers));
  let high = max(low, maxLayers);
  let layerCount = max(low, min(high, u32(mix(f32(high), f32(low), abs(v.z)) + 0.5)));
  let layerDepth = 1.0 / f32(layerCount);
  let rawSlope = v.xy / vz;
  let slopeLength = length(rawSlope);
  let boundedSlope = rawSlope * min(1.0, max(0.0, maxOffsetRatio) / max(slopeLength, 0.00001));
  let deltaUV = boundedSlope * scale / f32(layerCount);

  var currentUV = baseUV;
  var currentDepth = 0.0;
  var previousUV = baseUV;
  var previousDepth = 0.0;
  // Input is physical height: white/exposed=1, black/recessed=0. Ray depth is
  // measured downward from the top of the relief volume, so intersect against
  // surfaceDepth = 1-height (not height itself).
  var previousSurfaceDepth = 1.0 - textureSampleLevel(heightTexture, heightSampler, baseUV, 0.0).r;
  for (var i = 0u; i < layerCount; i = i + 1u) {
    currentUV = currentUV - deltaUV;
    currentDepth = currentDepth + layerDepth;
    let surfaceDepth = 1.0 - textureSampleLevel(heightTexture, heightSampler, currentUV, 0.0).r;
    if (surfaceDepth < currentDepth) {
      let afterDepth = surfaceDepth - currentDepth;
      let beforeDepth = previousSurfaceDepth - previousDepth;
      let denominator = afterDepth - beforeDepth;
      let weight = select(0.5, clamp(afterDepth / denominator, 0.0, 1.0), abs(denominator) > 0.00001);
      return mix(currentUV, previousUV, weight);
    }
    previousUV = currentUV;
    previousDepth = currentDepth;
    previousSurfaceDepth = surfaceDepth;
  }
  return currentUV;
}
`;

/** Bounded direct-light visibility ray from the POM hit toward one light. */
export const POM_SELF_SHADOW_WGSL = /* wgsl */ `
fn pomSelfShadow(
  heightTexture: texture_2d<f32>, heightSampler: sampler,
  hitUV: vec2f, lightDir: vec3f, heightScale: f32, maxOffsetRatio: f32,
  requestedSteps: u32, bias: f32
) -> f32 {
  let l = normalize(lightDir);
  if (l.z <= 0.001 || heightScale <= 0.0) { return 1.0; }
  let hitHeight = clamp(textureSampleLevel(heightTexture, heightSampler, hitUV, 0.0).r, 0.0, 1.0);
  let remainingHeight = 1.0 - hitHeight;
  if (remainingHeight <= bias) { return 1.0; }
  let steps = max(1u, min(requestedSteps, 16u));
  let rawSlope = l.xy / max(l.z, 0.001);
  let slopeLength = length(rawSlope);
  let boundedSlope = rawSlope * min(1.0, max(0.0, maxOffsetRatio) / max(slopeLength, 0.00001));
  let uvStep = boundedSlope * heightScale * remainingHeight / f32(steps);
  let heightStep = remainingHeight / f32(steps);
  var rayUV = hitUV;
  var rayHeight = hitHeight;
  for (var i = 0u; i < steps; i = i + 1u) {
    rayUV = rayUV + uvStep;
    rayHeight = rayHeight + heightStep;
    let terrainHeight = textureSampleLevel(heightTexture, heightSampler, rayUV, 0.0).r;
    if (terrainHeight > rayHeight + bias) { return 0.0; }
  }
  return 1.0;
}
`;

export function assertPomGeneratedWgsl(source: string): void {
  const fragment = source.lastIndexOf('@fragment');
  if (fragment < 0) throw new Error('POM shader contract: fragment entry point missing');
  const body = source.slice(fragment);
  const firstMarch = body.indexOf('pomMarchUV(');
  if (firstMarch < 0) throw new Error('POM shader contract: march invocation missing');
  if (body.indexOf('pomMarchUV(', firstMarch + 1) >= 0) {
    throw new Error('POM shader contract: march must execute exactly once');
  }
  const firstSample = body.indexOf('vtSampleFromLevel(');
  if (firstSample < 0 || firstMarch > firstSample) {
    throw new Error('POM shader contract: VT sampled before displaced UV initialization');
  }
  const marchLineEnd = body.indexOf('\n', firstMarch);
  const marchLine = body.slice(body.lastIndexOf('\n', firstMarch) + 1, marchLineEnd < 0 ? body.length : marchLineEnd);
  if (!marchLine.includes('mat3x3') || marchLine.includes('TBNViewMatrix')) {
    throw new Error('POM shader contract: view ray must use geometric TBN without normal-map dependency');
  }
  let samples = 0, cursor = 0;
  while ((cursor = body.indexOf('vtSampleFromLevel(', cursor)) >= 0) { samples++; cursor += 18; }
  if (samples !== 3) throw new Error(`POM shader contract: expected 3 linked PBR samples, got ${samples}`);
}

export interface PomReferenceResult {
  u: number;
  v: number;
  depth: number;
  layers: number;
  samples: number;
  hit: boolean;
}

export function pomLayerCount(viewZ: number, minLayers: number, maxLayers: number): number {
  const low = Math.max(1, Math.min(minLayers, maxLayers)) | 0;
  const high = Math.max(low, maxLayers) | 0;
  return Math.max(low, Math.min(high, Math.floor(high + (low - high) * Math.abs(viewZ) + 0.5)));
}

/** Apply visibility to only the contribution added by the current light. */
export function applyDirectLightVisibility(
  accumulatedBefore: number,
  accumulatedAfter: number,
  visibility: number,
): number {
  return accumulatedBefore + (accumulatedAfter - accumulatedBefore) * visibility;
}

export function pomDistanceFade(distance: number, maxDistance: number): number {
  if (!(maxDistance > 0)) return 1;
  if (distance >= maxDistance) return 0;
  const start = maxDistance * 0.65;
  if (distance <= start) return 1;
  const x = Math.max(0, Math.min(1, (distance - start) / (maxDistance - start)));
  return 1 - x * x * (3 - 2 * x);
}

/** Allocation-free CPU oracle matching `pomMarchUV` for deterministic tests. */
export function marchPomReference(
  sampleHeight: (u: number, v: number) => number,
  baseU: number,
  baseV: number,
  viewX: number,
  viewY: number,
  viewZ: number,
  heightScale: number,
  maxOffsetRatio: number,
  minLayers: number,
  maxLayers: number,
  maxDistance: number,
  viewDistance: number,
  out: PomReferenceResult,
): PomReferenceResult {
  out.u = baseU; out.v = baseV; out.depth = 0; out.layers = 0; out.samples = 0; out.hit = false;
  const fade = pomDistanceFade(viewDistance, maxDistance);
  const scale = heightScale * fade;
  if (!(scale > 0.00001)) return out;
  const length = Math.hypot(viewX, viewY, viewZ);
  if (!(length > 0)) return out;
  const x = viewX / length, y = viewY / length, z = viewZ / length;
  const layers = pomLayerCount(z, minLayers, maxLayers);
  out.layers = layers;
  const layerDepth = 1 / layers;
  const denominatorZ = Math.max(Math.abs(z), 0.001);
  const rawSlopeU = x / denominatorZ, rawSlopeV = y / denominatorZ;
  const slopeLength = Math.hypot(rawSlopeU, rawSlopeV);
  const slopeScale = Math.min(1, Math.max(0, maxOffsetRatio) / Math.max(slopeLength, 0.00001));
  const deltaU = rawSlopeU * slopeScale * scale / layers;
  const deltaV = rawSlopeV * slopeScale * scale / layers;
  let currentU = baseU, currentV = baseV, currentDepth = 0;
  let previousU = baseU, previousV = baseV, previousDepth = 0;
  let previousSurfaceDepth = 1 - Math.max(0, Math.min(1, sampleHeight(baseU, baseV)));
  out.samples = 1;
  for (let index = 0; index < layers; index++) {
    currentU -= deltaU; currentV -= deltaV; currentDepth += layerDepth;
    const surfaceDepth = 1 - Math.max(0, Math.min(1, sampleHeight(currentU, currentV)));
    out.samples++;
    if (surfaceDepth < currentDepth) {
      const afterDepth = surfaceDepth - currentDepth;
      const beforeDepth = previousSurfaceDepth - previousDepth;
      const divisor = afterDepth - beforeDepth;
      const weight = Math.abs(divisor) > 0.00001
        ? Math.max(0, Math.min(1, afterDepth / divisor)) : 0.5;
      out.u = currentU * (1 - weight) + previousU * weight;
      out.v = currentV * (1 - weight) + previousV * weight;
      out.depth = currentDepth * (1 - weight) + previousDepth * weight;
      out.hit = true;
      return out;
    }
    previousU = currentU; previousV = currentV; previousDepth = currentDepth;
    previousSurfaceDepth = surfaceDepth;
  }
  out.u = currentU; out.v = currentV; out.depth = currentDepth;
  return out;
}
