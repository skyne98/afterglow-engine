# Virtual Texturing Algorithms — Validated Reference

Prototype: `prototype/vt/vt.ts` (run: `bun run prototype/vt/vt.ts`)

All algorithms cross-checked against source code from:
- **[SHLOM]** shlomnissan/virtual-textures (C++/OpenGL, 2025)
- **[IDTECH]** id Software "Software Virtual Textures" paper (van Waveren, 2012)
- **[ELFRA]** elfrank/virtual-texturing (Three.js WebGL, r57)
- **[BARRE]** Sean Barrett "Sparse Virtual Textures" (GDC 2008)

## Test Results

```
Test 1: page table entry pack/unpack [SHLOM format]           ✓
Test 2: mip level computation [SHLOM ComputeMipLevel]        ✓
Test 3: LRU eviction [SHLOM page_cache.cpp]                   ✓
Test 4: pinned mips never evicted [SHLOM kMinPinnedLod]       ✓
Test 5: address translation [SHLOM material.frag]             ✓
Test 6: fallback to coarser mip [SHLOM fallback loop]          ✓
Test 7: border texels [IDTECH Section 3.2]                    ✓
Test 8: progressive loading [ELFRA/IDTECH]                     ✓
Test 9: full pipeline [IDTECH Section 5.1]                     ✓ (max diff: 0.0)
Test 10: camera movement (eviction + reload)                  ✓
```

---

## Algorithm 1: Page Table Entry Format

**Source:** [SHLOM] `page_manager.cpp`, `material.frag`

Bit-packed u32 entry:
```
bit 0:      resident flag (1 = page in atlas)
bits 1-8:   physical page X (0-255)
bits 9-16:  physical page Y (0-255)
bits 17-21: mip level (0-31, added for prototype)
```

**Pack:**
```ts
entry = (resident ? 1 : 0) | ((physX & 0xFF) << 1) | ((physY & 0xFF) << 9) | ((mip & 0x1F) << 17);
```

**Unpack (in shader):**
```glsl
bool isResident = (entry & 1u) != 0u;
uint physX = (entry >> 1) & 0xFFu;
uint physY = (entry >> 9) & 0xFFu;
```

**[SHLOM] page_manager.cpp:**
```cpp
auto entry = uint32_t { 0x1 | ((req.slot.x & 0xFFu) << 1) | ((req.slot.y & 0xFFu) << 9) };
```

---

## Algorithm 2: Mip Level Computation (Feedback Shader)

**Source:** [SHLOM] `feedback.frag`, [IDTECH] Appendix B, [ELFRA] `Shaders.js`

```
dx = dFdx(uv) * effectiveSize
dy = dFdy(uv) * effectiveSize
texelFootprint = max(dot(dx, dx), dot(dy, dy))
mipLevel = 0.5 * log2(max(texelFootprint, 1e-8))
```

The `0.5 * log2(d)` is equivalent to `log2(length(dx))` — the log2 of the
texel footprint size.

**Critical: Feedback buffer resolution compensation** ([SHLOM] `u_BufferScreenRatio`):

The feedback buffer is at 1/8 screen resolution. Each feedback pixel covers
8× more UV space than a render pixel, making derivatives 8× larger and mip
levels 3 too high. To compensate:

```
effectiveSize = virtualSize * feedbackScale  // = virtualSize * (1/8)
```

This makes the feedback mip level match what the full-resolution render needs.

**[SHLOM] feedback.frag:**
```glsl
vec2 effective_size = u_VirtualSize * u_BufferScreenRatio;
uint mip_level = uint(clamp(ComputeMipLevel(effective_size, v_TexCoord), ...));
```

**[IDTECH] Appendix B:**
```glsl
float2 texcoords = fragment.texcoord0.xy * virtTexelsWide;
float2 dx = ddx(texcoords);
float2 dy = ddy(texcoords);
float maxLod = 0.5 * log2(max(dot(dx,dx), dot(dy,dy)));
```

**[ELFRA] Shaders.js** (has `-1.0` offset for border compensation):
```glsl
return max(0.5 * log2(d) - 1.0, 0.0);
```

We use the [SHLOM]/[IDTECH] formula (no -1.0 offset).

---

## Algorithm 3: Page Table Lookup + Fallback

**Source:** [SHLOM] `material.frag`, [IDTECH] Section 3.1

When sampling the virtual texture, the shader:
1. Computes the desired mip level from screen-space derivatives
2. Looks up the page table at that mip level
3. If the page is NOT resident, walks up to coarser mip levels
4. Uses the first resident page found (may be coarser = blurrier)

```
for mip = desiredMip to maxMip:
    pageGrid = virtualPages >> mip
    pageX = floor(uv * pageGrid)
    pageY = floor(uv * pageGrid)
    entry = pageTable[mip][pageY][pageX]
    if entry.isResident:
        return (entry, mip)
return null  // shouldn't happen if coarsest mip is pinned
```

**[SHLOM] material.frag:**
```glsl
for (; mip_level <= max_level; ++mip_level) {
    curr_page_grid = max(u_PageGrid * exp2(-float(mip_level)), vec2(1.0));
    page_coords = floor(v_TexCoord * curr_page_grid);
    entry = texelFetch(u_PageTable, ivec2(page_coords), mip_level).r;
    if ((entry & 1u) != 0u) { is_resident = true; break; }
}
```

**[IDTECH] Section 3.1:**
"the address translation falls back to a page from a coarser mip level because
the texture page for the desired finer mip level is not yet available"

---

## Algorithm 4: Address Translation (Virtual UV → Atlas UV)

**Source:** [SHLOM] `material.frag`, [IDTECH] Appendix A

Given a resident page at (physX, physY) at mip level M:

1. Compute local UV within the page: `localUV = fract(uv * pageGridAtMip)`
2. Compute slot origin in atlas: `slotOrigin = (physX, physY) * slotSize`
3. Compute sample position: `sample = slotOrigin + border + localUV * pageSize`
4. Convert to atlas UV: `atlasUV = sample / atlasSize`

**Slot layout:** `[BORDER][pageSize payload][BORDER]` = slotSize

**[SHLOM] material.frag:**
```glsl
ivec2 physical_page = ivec2((entry >> 1) & 0xFFu, (entry >> 9) & 0xFFu);
vec2 local_uv = fract(v_TexCoord * curr_page_grid);
vec2 page_origin = vec2(physical_page) * (u_PageSize + u_PagePadding);
vec2 half_padding = u_PagePadding * 0.5;
vec2 sample_texel = page_origin + half_padding + local_uv * u_PageSize;
vec2 atlas_uv = sample_texel / u_AtlasSize;
```

Note: [SHLOM] uses `half_padding = padding * 0.5` because their padding is TOTAL
(4 texels = 2 per side). We use `PAGE_BORDER = 4` per side (id Software approach),
so offset = `PAGE_BORDER` directly.

---

## Algorithm 5: Physical Atlas + LRU Cache

**Source:** [SHLOM] `page_cache.cpp`, [IDTECH] Section 5.2

The physical atlas is a fixed-size texture divided into page slots. When a new
page needs to be loaded and the atlas is full, the LRU (Least Recently Used)
page is evicted.

**LRU structure:**
- `lruList`: doubly-linked list, front = MRU, back = LRU
- `lruMap`: page key → index in list (O(1) lookup)
- `freeSlots`: list of unused slot indices
- `pinnedMips`: set of mip levels that are never evicted

**Acquire slot:**
1. If page already resident → return existing slot
2. If free slot available → return it
3. Walk LRU from back, skip pinned pages, evict first non-pinned

**Touch:** Move page to front of LRU list (skip if pinned).

**Commit:** Write page data to atlas texture at slot position, add to LRU front.

**[SHLOM] page_cache.cpp Acquire():**
```cpp
// 1. Already resident?
if (req_to_slot_.find(request) != req_to_slot_.end())
    return { .slot = it->second, .evicted = nullopt };
// 2. Free slot?
if (!free_slots_.empty()) {
    auto slot = free_slots_.back();
    free_slots_.pop_back();
    return { .slot = slot, .evicted = nullopt };
}
// 3. Evict LRU (skip pinned)
auto it = lru_list_.rbegin();
while (it != lru_list_.rend()) {
    if (it->lod < kMinPinnedLod) break;
    ++it;
}
```

**[IDTECH] Section 5.2:**
"The priority is first based on the LOD level of the page such that finer mips
will be replaced first. Next, the page priority for replacement increases as the
number of rendered frames increases since the page was last used."

Note: [SHLOM] uses pure LRU. [IDTECH] uses mip-priority + LRU. We follow [SHLOM].

---

## Algorithm 6: Border Texel Generation

**Source:** [IDTECH] Section 3.2, Section 4.1

Each physical page has a border of texels around the payload. Border texels
replicate data from adjacent virtual pages so that bilinear filtering at page
boundaries produces correct results.

**Layout:** `[4px border][128px payload][4px border]` = 136px slot

**Generation:** For border texels at the edge of the virtual texture, clamp to
the edge texel (standard texture clamping behavior).

**[IDTECH] Section 3.2:**
"In order to properly support hardware bi-linear filtering, each physical texture
page must have a border of texels around it."

**[IDTECH] Section 4.1:**
"a 4-texel border is typically used around each physical page. The border
texels need not be stored on disk, but as a practical matter, it is far less
complicated to have pages be fully independent."

---

## Algorithm 7: Feedback Analysis

**Source:** [IDTECH] Section 3.4, [SHLOM] `page_manager.cpp`

The feedback buffer contains packed page requests from the GPU. The CPU:

1. **Deduplicate**: Walk feedback buffer, collect unique page requests
2. **Touch**: Mark all resident pages seen in feedback as recently used (LRU)
3. **Load**: For non-resident pages, load data → acquire slot → commit → update page table

**[SHLOM] IngestFeedback():**
```cpp
std::set<PageRequest> requests;
for (auto packed : feedback) {
    if ((packed & (1u << 31)) == 0u) continue;  // skip invalid
    packed &= ~(1u << 31);                       // strip valid bit
    requests.emplace(packed & 0x1Fu, (packed >> 5) & 0xFFu, (packed >> 13) & 0xFFu);
}
for (auto request : requests) {
    page_cache_.Touch(request);
    if (!page_tables_.IsResident(request) && !requests_.contains(request))
        RequestPage(request);
}
page_tables_.SyncTables();
```

**[IDTECH] Section 3.4:**
"The feedback analysis walks the screen buffer and condenses the page
information into a list with unique pages."

---

## Algorithm 8: Oversubscription Handling

**Source:** [IDTECH] Section 3.5

When the atlas can't hold all requested pages (thrashing), the system
dynamically adjusts the feedback LOD bias to request coarser mips.

- Track resident pages seen in feedback
- If > high water mark (90% of atlas): increment LOD bias (back off detail)
- If < low water mark (50% of atlas): decrement LOD bias (add detail back)
- LOD bias is always clamped to >= 0

**[IDTECH] Section 3.5:**
"The number of resident pages that were seen in the previous frame's feedback
is tracked. If that number is greater than a high water mark, the system is
considered oversubscribed and the LOD bias used when generating feedback is
incremented."

---

## Algorithm 9: Pinned Mip Levels

**Source:** [SHLOM] `kMinPinnedLod`, [IDTECH] Section 4.5

The coarsest mip levels are always resident (pinned) and never evicted. This
ensures the shader always has a fallback page when fine pages aren't loaded.

**[SHLOM]:** `kMinPinnedLod = 4` (LODs >= 4 are pinned)
**Our prototype:** `PINNED_MIPS = {MAX_MIP, MAX_MIP - 1}` (coarsest 2 levels)

**[IDTECH] Section 4.5:**
"the texture page that represents the coarsest mip level of a virtual texture
is usually locked in the physical page textures to make sure there is always
some texture data to fall back to"

---

## Constants

| Parameter | Prototype | [SHLOM] | [IDTECH] (RAGE) |
|-----------|-----------|---------|-----------------|
| Page size | 128×128 | 512×512 | 128×128 |
| Border | 4 per side | 4 total (2/side) | 4 per side |
| Slot size | 136×136 | 516×516 | 128×128 |
| Atlas size | 1088×1088 | 4096×4096 | 4096×4096 |
| Atlas slots | 8×8=64 | 8×8=64 | 32×32=1024 |
| Virtual size | 4096×4096 | 8192×8192 | 120K×120K |
| Virtual pages | 32×32 | 16×16 | 1024×1024 |
| Max mip | 5 | 4 | 10 |
| Feedback res | 1/8 screen | configurable | 1/10 screen |
| Pinned mips | 2 coarsest | 1 coarsest | 1 coarsest |
