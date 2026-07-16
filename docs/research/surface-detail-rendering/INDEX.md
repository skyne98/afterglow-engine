# Sources — Surface Detail / "Flat Walls Not Flat"

## Deep dive + implementation references (the recommended technique)
- **`DEEP-DIVE-PPOM-RCS.md`** — implementation-grade spec: Prism POM + Relaxed
  Cone Stepping + binary search + self-shadow. Math, WGSL pseudocode,
  precompute, iGPU perf budget, artifacts, WebGPU integration.
- **`IMPLEMENTATIONS-REFERENCE.md`** — catalog of proven working
  implementations (SkyeShark three.js WebGPU, GPU Gems 3 CD, Bundas102 robust
  CSM 2024, bbalazs prism+CSM, POM self-shadow shaders, cone-map generators).
- **`impl-references/`** — saved primary shader sources for direct porting.

## Survey
- `README.md` — the full SOTA ladder (cheap raster → ray-traced displacement).

## Primary sources (full text fetched)
| File | Source | URL |
|------|--------|-----|
| prism-pom-dachsbacher-tatarchuk-2007.md | Dachsbacher & Tatarchuk 2007, Prism POM with Accurate Silhouette (HAL) | https://inria.hal.science/inria-00606806 |
| gpu-gems3-ch18-relaxed-cone-stepping.txt | GPU Gems 3 ch.18 Relaxed Cone Stepping — full text + precompute shader | https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-18-relaxed-cone-stepping-relief-mapping |
| learnopengl-parallax-shader.txt | LearnOpenGL Parallax Mapping — concrete steep/POM GLSL | https://learnopengl.com/Advanced-Lighting/Parallax-Mapping |
| displacement-review-blekinge-2012.txt | Review of Displacement Mapping Techniques (Blekinge 2012) | https://www.diva-portal.org/smash/get/diva2:831762/FULLTEXT01.pdf |

## Per-topic source notes (synthesis)
| File | Topic |
|------|-------|
| pdm-hoetzlein2025.md | Projective Displacement Mapping (Hoetzlein 2025) — ray-traced SOTA; normal-correction + parallel-prism ideas reused |
| gpu-gems-relaxed-cone-stepping.md | CSM / RCS / anisotropic / ARCSM (2025) / Robust CSM (2024) overview |
| nv-micromesh.md | NVIDIA Displacement Micro-Map (RTX 40 HW RT) |
| raytraced-displacement.md | D-BVH (Thonat 2021) / RMIP (2023) / Ogaki nonlinear (2023) / Haydel LOD (2023) |
| learnopengl-parallax.md | parallax → steep → POM → relief ladder (summary) |
