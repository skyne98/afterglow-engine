// Bounded low-core parallax occlusion mapping for resident height textures.

/**
 * Adaptive 8–32-style POM march with interpolation and distance fade.
 * No silhouette, self-shadow, or secondary relief pass.
 */
export const POM_UV_WGSL = /* wgsl */ `
fn pomMarchUV(
  heightTexture: texture_2d<f32>, heightSampler: sampler,
  baseUV: vec2f, viewDir: vec3f, heightScale: f32,
  minLayers: u32, maxLayers: u32, maxDistance: f32, viewDistance: f32
) -> vec2f {
  if (heightScale <= 0.0 || maxDistance <= 0.0 || viewDistance >= maxDistance) {
    return baseUV;
  }
  let fade = 1.0 - smoothstep(maxDistance * 0.65, maxDistance, viewDistance);
  let scale = heightScale * fade;
  if (scale <= 0.00001) { return baseUV; }

  let v = normalize(viewDir);
  let vz = max(abs(v.z), 0.001);
  let low = max(1u, min(minLayers, maxLayers));
  let high = max(low, maxLayers);
  let layerCount = max(low, min(high, u32(mix(f32(high), f32(low), abs(v.z)) + 0.5)));
  let layerDepth = 1.0 / f32(layerCount);
  let deltaUV = v.xy * scale / (vz * f32(layerCount));

  var currentUV = baseUV;
  var currentDepth = 0.0;
  var previousUV = baseUV;
  var previousDepth = 0.0;
  var previousHeight = textureSampleLevel(heightTexture, heightSampler, baseUV, 0.0).r;
  for (var i = 0u; i < layerCount; i = i + 1u) {
    currentUV = currentUV - deltaUV;
    currentDepth = currentDepth + layerDepth;
    let sampledHeight = textureSampleLevel(heightTexture, heightSampler, currentUV, 0.0).r;
    if (sampledHeight < currentDepth) {
      let afterDepth = sampledHeight - currentDepth;
      let beforeDepth = previousHeight - previousDepth;
      let denominator = afterDepth - beforeDepth;
      let weight = select(0.5, clamp(afterDepth / denominator, 0.0, 1.0), abs(denominator) > 0.00001);
      return mix(currentUV, previousUV, weight);
    }
    previousUV = currentUV;
    previousDepth = currentDepth;
    previousHeight = sampledHeight;
  }
  return currentUV;
}
`;
