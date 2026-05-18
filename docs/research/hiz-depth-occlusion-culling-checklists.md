# Hi-Z Depth / HZB Occlusion Culling Checklists

Companion checklist and source index for [hiz-depth-occlusion-culling.md](hiz-depth-occlusion-culling.md).

## Correctness Checklist

If any of these fail, the system is suspicious:

- wrong reduction operator for the depth convention
- ignoring odd mip-edge texels
- testing center depth instead of conservative nearest depth
- using a proxy box that extends outside opaque coverage
- masking/alpha/displacement mismatch between depth proxy path and real main-pass coverage
- treating near-plane-straddling objects as culled
- using previous-frame HZB without a disocclusion strategy

## Efficiency Checklist

After correctness is stable, optimize in this order:

1. frustum-cull before HZB
2. sort or bucket proxies for cheap depth rendering
3. move from per-mip dispatch to SPD-style downsampling if needed
4. reduce proxy depth resolution only after measuring culling quality loss
5. consider two-phase temporal culling later if same-frame selective proxies are not enough

## Recommended Debug Views

You will want all of these:

- occluder proxy visualization in world space
- proxy depth buffer
- selected HZB mip
- occludee bounds overlay
- per-instance visibility state
- total instance counter
- frustum rejected counter
- HZB rejected counter
- rendered instance counter

For debugging false culls, add a mode that forces a selected instance visible and shows:

- its projected rectangle
- chosen mip
- sampled HZB values
- compared nearest depth

## Suggested Policy for This Engine

For `afterglow-engine`, the most defensible policy is:

- allow every model to author an **occludee bound**
- allow selected models to author an **occluder proxy**
- treat occluder proxies as **opt-in**, not automatic
- keep proxies for **large, opaque, static, solid** assets
- keep thin, masked, hollow, or animated assets out of the occluder set until proven safe

This keeps the system conservative and predictable for both engineering and content.

## Sources

### Primary / Highly Authoritative

- Greene, Kass, Miller, *Hierarchical Z-Buffer Visibility* (SIGGRAPH 1993)  
  https://www.cs.cmu.edu/afs/cs/academic/class/15869-f11/www/readings/greene93_hierarchicalz.pdf

- AMD / SIGGRAPH 2008 course notes, *March of the Froblins*  
  https://advances.realtimerendering.com/s2008/Siggraph2008-AdvancesInRealTimeRendering-CourseNotes.pdf

- Epic Games, *Nanite: A Deep Dive* (SIGGRAPH 2021 Advances in Real-Time Rendering)  
  https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf

- Ubisoft, *GPU-Driven Rendering Pipelines* (SIGGRAPH 2015)  
  https://www.advances.realtimerendering.com/s2015/aaltonenhaar_siggraph2015_combined_final_footer_220dpi.pdf

- NVIDIA, *Visualizing Depth Precision*  
  https://developer.nvidia.com/blog/visualizing-depth-precision/

- AMD GPUOpen, FidelityFX Single Pass Downsampler  
  https://gpuopen.com/manuals/fidelityfx_sdk/techniques/single-pass-downsampler/

- Microsoft Learn, `earlydepthstencil`  
  https://learn.microsoft.com/en-us/windows/win32/direct3dhlsl/sm5-attributes-earlydepthstencil

- Microsoft Learn, Conservative Rasterization  
  https://learn.microsoft.com/en-us/windows/win32/direct3d11/conservative-rasterization

- NVIDIA GPU Gems, Chapter 29, *Efficient Occlusion Culling*  
  https://developer.nvidia.com/gpugems/gpugems/part-v-performance-and-practicalities/chapter-29-efficient-occlusion-culling

- NVIDIA GPU Gems 2, Chapter 6, *Hardware Occlusion Queries Made Useful*  
  https://developer.nvidia.com/gpugems/gpugems2/part-i-geometric-complexity/chapter-6-hardware-occlusion-queries-made-useful

- NVIDIA GPU Gems 3, Chapter 19, *Deferred Shading in Tabula Rasa*  
  https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-19-deferred-shading-tabula-rasa

- Unity URP deferred pass order  
  https://docs.unity.cn/6000.0/Documentation/Manual/urp/rendering/render-passes-deferred.html

- Unity URP depth-only pass guidance  
  https://docs.unity3d.com/ja/current/Manual/urp/writing-shaders-urp-depth-only.html

- Unity occlusion culling setup  
  https://docs.unity.cn/6000.1/Documentation/Manual/occlusion-culling-getting-started.html

- Unity Skinned Mesh Renderer bounds behavior  
  https://docs.unity.cn/Manual/class-SkinnedMeshRenderer.html

- Bevy `OcclusionCulling` docs  
  https://docs.rs/bevy/latest/bevy/render/experimental/occlusion_culling/struct.OcclusionCulling.html

### Useful Supplemental Implementation Writeups

- RasterGrid, *Hierarchical-Z map based occlusion culling*  
  https://www.rastergrid.com/blog/2010/10/hierarchical-z-map-based-occlusion-culling/

- Self Shadow, *Practical, Dynamic Visibility for Games*  
  https://blog.selfshadow.com/publications/practical-visibility/

- ARM OpenGL ES SDK, *Occlusion culling with a hierarchical depth buffer*  
  https://arm-software.github.io/opengl-es-sdk-for-android/occlusion_culling.html

- Mike Turitzin, *Hierarchical Depth Buffers*  
  https://miketuritzin.com/post/hierarchical-depth-buffers/
