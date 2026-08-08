// ============================================================================
// WGSL Shaders (for the material system)
// ============================================================================

/**
 * The VT sampling shader. Override material.colorNode with this.
 *
 * Source: [SHLOM] material.frag, validated in prototype.
 * Uses wgslFn() for pure WGSL — TSL handles binding plumbing.
 */
export const VT_SAMPLE_WGSL = /* wgsl */ `
fn vtSample(
  pageTable: texture_2d<u32>,
  atlas: texture_2d<f32>,
  atlasSampler: sampler,
  uv: vec2f,
  virtualSize: vec2f,
  pageGrid: vec2f,
  pageSize: f32,
  pageBorder: f32,
  atlasSize: vec2f,
  maxMip: f32,
  textureMaxMip: f32,
  mipBias: f32,
  filterMode: u32,
  addressMode: u32
) -> vec4f {
  // 0 = clamp, 1 = repeat, 2 = mirrored repeat.
  var addressed_uv = clamp(uv, vec2f(0.0), vec2f(0.99999994));
  if (addressMode == 1u) {
    addressed_uv = fract(uv);
  } else if (addressMode == 2u) {
    let period = uv - floor(uv * 0.5) * 2.0;
    addressed_uv = select(period, 2.0 - period, period > vec2f(1.0));
    addressed_uv = clamp(addressed_uv, vec2f(0.0), vec2f(0.99999994));
  }

  // Compute desired mip level from the original continuous derivatives.
  let dx = dpdx(uv * virtualSize);
  let dy = dpdy(uv * virtualSize);
  let texel_footprint = max(dot(dx, dx), dot(dy, dy));
  let mip_float = clamp(
    0.5 * log2(max(texel_footprint, 1e-8)) + mipBias,
    0.0,
    textureMaxMip
  );
  let desired_level = i32(mip_float);

  // Mips below 128x128 share one pinned physical slot. The entry is stored in
  // the otherwise-unused x=1 texel of the terminal page-table row.
  if (desired_level > i32(maxMip)) {
    var tail_offset = 0.0;
    for (var level = 0; level < i32(maxMip); level = level + 1) {
      tail_offset += max(1.0, ceil(pageGrid.y / exp2(f32(level))));
    }
    let tail_entry = textureLoad(pageTable, vec2i(1, i32(tail_offset)), 0).r;
    if ((tail_entry & 1u) != 0u) {
      let delta = desired_level - i32(maxMip);
      var rect_origin = vec2f(0.0);
      if (delta == 2) { rect_origin = vec2f(72.0, 0.0); }
      else if (delta == 3) { rect_origin = vec2f(112.0, 0.0); }
      else if (delta == 4) { rect_origin = vec2f(72.0, 40.0); }
      else if (delta == 5) { rect_origin = vec2f(88.0, 40.0); }
      else if (delta == 6) { rect_origin = vec2f(100.0, 40.0); }
      else if (delta >= 7) { rect_origin = vec2f(110.0, 40.0); }
      let tail_size = max(vec2f(1.0), floor(virtualSize / exp2(f32(desired_level))));
      let tail_x = (tail_entry >> 1) & 0xFFu;
      let tail_y = (tail_entry >> 9) & 0xFFu;
      let slot_origin = vec2f(f32(tail_x), f32(tail_y)) * (pageSize + pageBorder * 2.0);
      let tail_texel = slot_origin + rect_origin + pageBorder + addressed_uv * tail_size;
      let tail_uv = tail_texel / atlasSize;
      let tail_scale = tail_size / atlasSize;
      if (filterMode == 1u) {
        return textureLoad(atlas, vec2i(clamp(floor(tail_texel), vec2f(0.0), atlasSize - 1.0)), 0);
      }
      return textureSampleGrad(atlas, atlasSampler, tail_uv, dpdx(uv) * tail_scale, dpdy(uv) * tail_scale);
    }
  }

  var mip_level = min(desired_level, i32(maxMip));
  let max_level = i32(maxMip);

  // Walk from desired mip up, looking for resident page
  var is_resident = false;
  var entry = 0u;
  var curr_page_grid = vec2f(0.0);
  var curr_mip_size = virtualSize;
  var page_coords = vec2i(0);

  for (var m = mip_level; m <= max_level; m = m + 1) {
    let mip_scale = exp2(-f32(m));
    curr_page_grid = max(ceil(pageGrid * mip_scale), vec2f(1.0));
    curr_mip_size = max(floor(virtualSize * mip_scale), vec2f(1.0));
    page_coords = vec2i(min(floor(addressed_uv * curr_mip_size / pageSize), curr_page_grid - 1.0));
    var mip_offset = 0.0;
    for (var level = 0; level < m; level = level + 1) {
      mip_offset += max(1.0, ceil(pageGrid.y / exp2(f32(level))));
    }
    entry = textureLoad(pageTable, vec2i(page_coords.x, page_coords.y + i32(mip_offset)), 0).r;
    if ((entry & 1u) != 0u) {
      is_resident = true;
      mip_level = m;
      break;
    }
  }

  if (!is_resident) {
    return vec4f(0.5, 0.5, 0.5, 1.0);
  }

  // Compute physical atlas UV
  let physX = (entry >> 1) & 0xFFu;
  let physY = (entry >> 9) & 0xFFu;
  let local_texel = addressed_uv * curr_mip_size - vec2f(page_coords) * pageSize;
  let page_origin = vec2f(f32(physX), f32(physY)) * (pageSize + pageBorder * 2.0);
  let sample_texel = page_origin + pageBorder + local_texel;
  let atlas_uv = sample_texel / atlasSize;

  // Atlas-space gradients preserve anisotropy without allowing the GPU to
  // derive across an unrelated neighboring physical slot.
  let gradient_scale = curr_mip_size / atlasSize;
  let atlas_dx = dpdx(uv) * gradient_scale;
  let atlas_dy = dpdy(uv) * gradient_scale;
  if (filterMode == 1u) {
    return textureLoad(atlas, vec2i(clamp(floor(sample_texel), vec2f(0.0), atlasSize - 1.0)), 0);
  }
  return textureSampleGrad(atlas, atlasSampler, atlas_uv, atlas_dx, atlas_dy);
}
`;

/** Select one channel's requested mip from stable base-UV derivatives. */
export const VT_DESIRED_MIP_WGSL = /* wgsl */ `
fn vtDesiredMip(
  gradientUV: vec2f,
  virtualSize: vec2f,
  textureMaxMip: f32,
  mipBias: f32
) -> f32 {
  let dx = dpdx(gradientUV * virtualSize);
  let dy = dpdy(gradientUV * virtualSize);
  let footprint = max(dot(dx, dx), dot(dy, dy));
  return clamp(0.5 * log2(max(footprint, 1e-8)) + mipBias, 0.0, textureMaxMip);
}
`;

/** Sample a displaced UV from one channel's requested level, walking to its own fallback. */
export const VT_SAMPLE_FROM_LEVEL_WGSL = /* wgsl */ `
fn vtSampleFromLevel(
  pageTable: texture_2d<u32>, atlas: texture_2d<f32>, atlasSampler: sampler,
  sampleUV: vec2f, gradientUV: vec2f,
  virtualSize: vec2f, pageGrid: vec2f, pageSize: f32, pageBorder: f32,
  atlasSize: vec2f, maxMip: f32, resolvedMip: f32, addressMode: u32
) -> vec4f {
  var addressedUV = clamp(sampleUV, vec2f(0.0), vec2f(0.99999994));
  if (addressMode == 1u) {
    addressedUV = fract(sampleUV);
  } else if (addressMode == 2u) {
    let period = sampleUV - floor(sampleUV * 0.5) * 2.0;
    addressedUV = select(period, 2.0 - period, period > vec2f(1.0));
    addressedUV = clamp(addressedUV, vec2f(0.0), vec2f(0.99999994));
  }

  let requested = i32(resolvedMip);
  let maxLevel = i32(maxMip);
  var entry = 0u;
  var selected = -1;
  var selectedPage = vec2i(0);
  var selectedSize = vec2f(1.0);
  if (requested <= maxLevel) {
    for (var mip = max(0, requested); mip <= maxLevel; mip = mip + 1) {
      let scale = exp2(-f32(mip));
      let grid = max(ceil(pageGrid * scale), vec2f(1.0));
      let mipSize = max(floor(virtualSize * scale), vec2f(1.0));
      let page = vec2i(min(floor(addressedUV * mipSize / pageSize), grid - 1.0));
      var offset = 0.0;
      for (var level = 0; level < mip; level = level + 1) {
        offset += max(1.0, ceil(pageGrid.y / exp2(f32(level))));
      }
      let candidate = textureLoad(pageTable, vec2i(page.x, page.y + i32(offset)), 0).r;
      if ((candidate & 1u) != 0u) {
        entry = candidate; selected = mip; selectedPage = page; selectedSize = mipSize;
        break;
      }
    }
  }

  if (selected >= 0) {
    let local = addressedUV * selectedSize - vec2f(selectedPage) * pageSize;
    let origin = vec2f(f32((entry >> 1) & 0xFFu), f32((entry >> 9) & 0xFFu)) * (pageSize + pageBorder * 2.0);
    let atlasUV = (origin + pageBorder + local) / atlasSize;
    let gradientScale = selectedSize / atlasSize;
    return textureSampleGrad(atlas, atlasSampler, atlasUV,
      dpdx(gradientUV) * gradientScale, dpdy(gradientUV) * gradientScale);
  }

  var tailOffset = 0.0;
  for (var level = 0; level < maxLevel; level = level + 1) {
    tailOffset += max(1.0, ceil(pageGrid.y / exp2(f32(level))));
  }
  let tailEntry = textureLoad(pageTable, vec2i(1, i32(tailOffset)), 0).r;
  if ((tailEntry & 1u) == 0u) { return vec4f(0.5, 0.5, 0.5, 1.0); }
  let tailMip = max(maxLevel + 1, requested);
  let delta = tailMip - maxLevel;
  var rectOrigin = vec2f(0.0);
  if (delta == 2) { rectOrigin = vec2f(72.0, 0.0); }
  else if (delta == 3) { rectOrigin = vec2f(112.0, 0.0); }
  else if (delta == 4) { rectOrigin = vec2f(72.0, 40.0); }
  else if (delta == 5) { rectOrigin = vec2f(88.0, 40.0); }
  else if (delta == 6) { rectOrigin = vec2f(100.0, 40.0); }
  else if (delta >= 7) { rectOrigin = vec2f(110.0, 40.0); }
  let tailSize = max(vec2f(1.0), floor(virtualSize / exp2(f32(tailMip))));
  let slot = vec2f(f32((tailEntry >> 1) & 0xFFu), f32((tailEntry >> 9) & 0xFFu)) * (pageSize + pageBorder * 2.0);
  let atlasUV = (slot + rectOrigin + pageBorder + addressedUV * tailSize) / atlasSize;
  let gradientScale = tailSize / atlasSize;
  return textureSampleGrad(atlas, atlasSampler, atlasUV,
    dpdx(gradientUV) * gradientScale, dpdy(gradientUV) * gradientScale);
}
`;

/**
 * The feedback shader. Renders to a low-res target, writing page IDs.
 *
 * Source: [SHLOM] feedback.frag, validated in prototype.
 */
export const VT_FEEDBACK_WGSL = /* wgsl */ `
fn vtFeedback(
  sampleUV: vec2f,
  gradientUV: vec2f,
  feedbackPixelScale: vec2f,
  virtualSize: vec2f,
  pageGrid: vec2f,
  maxMip: f32,
  qualityBias: f32,
  addressMode: u32,
  textureId: u32,
  viewDistance: f32,
  cameraNear: f32,
  cameraFar: f32
) -> vec2u {
  // Derivatives are measured per reduced-resolution feedback pixel. Convert
  // them back to physical display-pixel derivatives before selecting a mip.
  // Keeping gradientUV separate prevents repeat/POM discontinuities from
  // corrupting the screen-space footprint.
  let dx = dpdx(gradientUV * virtualSize) * feedbackPixelScale.x;
  let dy = dpdy(gradientUV * virtualSize) * feedbackPixelScale.y;
  let texel_footprint = max(dot(dx, dx), dot(dy, dy));
  let mip_level = u32(clamp(0.5 * log2(max(texel_footprint, 1e-8)) + qualityBias, 0.0, maxMip));

  var addressed_uv = clamp(sampleUV, vec2f(0.0), vec2f(0.99999994));
  if (addressMode == 1u) {
    addressed_uv = fract(sampleUV);
  } else if (addressMode == 2u) {
    let period = sampleUV - floor(sampleUV * 0.5) * 2.0;
    addressed_uv = select(period, 2.0 - period, period > vec2f(1.0));
    addressed_uv = clamp(addressed_uv, vec2f(0.0), vec2f(0.99999994));
  }
  let mip_scale = exp2(-f32(mip_level));
  let curr_page_grid = max(ceil(pageGrid * mip_scale), vec2f(1.0));
  let mip_size = max(floor(virtualSize * mip_scale), vec2f(1.0));
  let page_coords = min(floor(addressed_uv * mip_size / 128.0), curr_page_grid - 1.0);

  let safeNear = max(cameraNear, 1e-6);
  let safeFar = max(cameraFar, safeNear + 1e-6);
  let logRange = max(log2(safeFar / safeNear), 1e-6);
  let normalizedDistance = clamp(
    log2(max(viewDistance, safeNear) / safeNear) / logRange, 0.0, 1.0
  );
  let cameraCloseness = u32(round((1.0 - normalizedDistance) * 7.0));

  // RG32Uint: word 0 carries valid + 3-bit camera closeness + 6-bit mip + 11-bit X/Y;
  // word 1 carries the full virtual-texture identity.
  let packed = 0x80000000u |
               ((cameraCloseness & 0x7u) << 28) |
               (mip_level & 0x3Fu) |
               ((u32(page_coords.x) & 0x7FFu) << 6) |
               ((u32(page_coords.y) & 0x7FFu) << 17);
  return vec2u(packed, textureId);
}
`;
