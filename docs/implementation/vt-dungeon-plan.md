# Engine VT dungeon implementation

## Goal

A minimal first-person dungeon proving complete scanned PBR material sets through
the real offline and runtime virtual-texture pipeline.

## Completed

- [x] Fixed twelve-segment corridor/room layout
- [x] Circle-versus-wall collision with sliding
- [x] WASD, sprint, pointer-lock mouse look, reset and viewpoint keys
- [x] Stable programmatic pose, movement, look, stepping, idle and scenario API
- [x] Three scanned 8K materials shared across twelve wall instances
- [x] Generic pipeline-generated Basis `.big` cache under `/tmp`
- [x] AssetLoader range reads and TextureWorker GPU-format transcoding
- [x] Linked albedo, normal, roughness, and AO virtual material channels
- [x] One engine `VirtualTextureStore`, physical atlas and scheduler for all walls
- [x] Engine `VT_SAMPLE_WGSL` material on every wall
- [x] Shared procedural page module used by both 2D and 3D demos
- [x] 2D demo migrated from its manual cache to `VirtualTextureStore`
- [x] Three-viewpoint, independently launched CEF GPU regression
- [x] Material-group feedback, identity, RGBA layout, and capacity tests
- [x] Visual inspection at near-wall and corridor viewpoints

## Deliberate scope

The dungeon has no gameplay, dynamic lights, stairs, props or physics engine.
Visibility and mip requests come from the engine's reduced-resolution
`RG32Uint` GPU feedback pass. Fragment derivatives measure virtual texels per
screen pixel; the engine coarsens requests only if their unique working set
exceeds the device-sized atlas. There are no dungeon-specific visibility or mip
heuristics. Floor and ceiling are ordinary materials; only walls are virtual.
