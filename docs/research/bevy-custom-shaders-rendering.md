# Bevy 0.18 Custom Shaders & Rendering Pipeline

## 1. The Material Trait (High-Level API)

The easiest way to render custom stuff. Implement `bevy::pbr::Material` on your type:

```rust
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct MyMaterial {
    #[uniform(0)]
    color: LinearRgba,
    #[texture(1)]
    #[sampler(2)]
    color_texture: Option<Handle<Image>>,
}

impl Material for MyMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/my_material.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode { AlphaMode::Opaque }
    fn opaque_render_method(&self) -> OpaqueRendererMethod { OpaqueRendererMethod::Forward }
}
```

Add with `app.add_plugins(MaterialPlugin::<MyMaterial>::default())`. Spawn:
```rust
commands.spawn((
    Mesh3d(meshes.add(Cuboid::default())),
    MeshMaterial3d(materials.add(MyMaterial { .. })),
));
```

### AsBindGroup Derive

Field attributes:
- `#[uniform(N)]` — uniform buffer at binding N (type must impl `ShaderType`)
- `#[texture(N)]` — texture at binding N
- `#[sampler(N)]` — sampler at binding N
- `#[storage(N, read_only)]` — storage buffer at binding N
- `#[storage_texture(N, ...)]` — storage texture

### AlphaMode
`Opaque`, `Mask(f32)`, `Blend`, `Premultiplied`, `AlphaToCoverage`, `Add`, `Multiply`

### OpaqueRendererMethod
`Forward`, `Deferred`, `Auto`

---

## 2. ExtendedMaterial — Wrap StandardMaterial

Add custom data/shaders on top of PBR:

```rust
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
struct MyExtension {
    #[uniform(100)]  // start from 100 to avoid StandardMaterial's bindings (0-99)
    quantize_steps: u32,
}

impl MaterialExtension for MyExtension {
    fn fragment_shader() -> ShaderRef { "shaders/extended.wgsl".into() }
}

// Spawn with:
materials.add(ExtendedMaterial {
    base: StandardMaterial { base_color: RED.into(), ..default() },
    extension: MyExtension { quantize_steps: 4 },
});
```

---

## 3. Custom Render Pipelines (SpecializedMeshPipeline)

For full control — define your own `RenderPipelineDescriptor` while reusing Bevy's mesh/view infrastructure:

```rust
#[derive(Resource)]
struct MyPipeline { mesh_pipeline: MeshPipeline, shader: Handle<Shader> }

impl SpecializedMeshPipeline for MyPipeline {
    type Key = MeshPipelineKey;
    fn specialize(&self, key: Self::Key, layout: &MeshVertexBufferLayoutRef)
        -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError>
    {
        let vertex_layout = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?;
        let view_layout = self.mesh_pipeline.get_view_layout(key.into());
        Ok(RenderPipelineDescriptor {
            vertex: VertexState { shader: self.shader.clone(), buffers: vec![vertex_layout], ..default() },
            fragment: Some(FragmentState { shader: self.shader.clone(), targets: vec![Some(ColorTargetState { format: TextureFormat::bevy_default(), ..default() })], ..default() }),
            layout: vec![view_layout.main_layout.clone(), view_layout.empty_layout.clone(), self.mesh_pipeline.mesh_layouts.model_only.clone()],
            depth_stencil: Some(DepthStencilState { format: CORE_3D_DEPTH_FORMAT, depth_write_enabled: true, depth_compare: CompareFunction::GreaterEqual, ..default() }),
            ..default()
        })
    }
}
```

Register in the render sub-app with `SpecializedMeshPipelines<MyPipeline>`, queue entities into `Opaque3d` phase.

---

## 4. The Render Graph

A DAG of `Node`s and `SubGraph`s:
- `Node` trait — `run()` gets graph context, render context, world
- `ViewNode` trait — `run()` gets view query data + world, wrapped by `ViewNodeRunner`
- Nodes declare input/output `SlotInfo` (textures, buffers)
- Built-in sub-graph: `Core3d`
- Built-in node labels: `Node3d::MainOpaquePass`, `Node3d::Tonemapping`, `Node3d::EndMainPassPostProcessing`, etc.

```rust
render_app.add_render_graph_node::<ViewNodeRunner<MyNode>>(Core3d, MyLabel);
render_app.add_render_graph_edges(Core3d, (Node3d::Tonemapping, MyLabel, Node3d::EndMainPassPostProcessing));
```

---

## 5. Compute Shaders

Steps:
1. Define `ComputePipelineDescriptor` with entry point
2. Queue via `pipeline_cache.queue_compute_pipeline(desc)` → get `CachedComputePipelineId`
3. Create bind groups with `render_device.create_bind_group()`
4. Dispatch in a render graph `Node` via `begin_compute_pass().set_pipeline().dispatch_workgroups()`
5. Read back with `Readback::buffer(handle)` + `ReadbackComplete` observer

---

## 6. Post-Processing

Pattern: `ViewNode` + `ViewTarget::post_process_write()` + fullscreen triangle:

```rust
impl ViewNode for MyPostProcess {
    type ViewQuery = &'static ViewTarget;
    fn run(&self, _graph: &mut RenderGraphContext, render_context: &mut RenderContext,
           view_target: QueryItem<Self::ViewQuery>, world: &World) -> Result<(), NodeRunError>
    {
        let pp = view_target.post_process_write();
        // pp.source = current frame, pp.destination = output
        let bind_group = render_context.render_device().create_bind_group(
            "pp_bg", &layout, &BindGroupEntries::sequential((pp.source, &sampler, &uniform)));
        let mut pass = render_context.begin_tracked_render_pass(..);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1); // fullscreen triangle
    }
}
```

The `FullscreenShader` resource provides the vertex shader. Insert between `Node3d::Tonemapping` and `Node3d::EndMainPassPostProcessing`.

---

## 7. Available WGSL Imports

From `bevy_pbr`:
- `bevy_pbr::mesh_bindings` — `Mesh` struct, model matrix
- `bevy_pbr::mesh_view_bindings` — view uniforms, lights
- `bevy_pbr::mesh_view_types` — `View`, `PointLight`, `DirectionalLight` structs
- `bevy_pbr::mesh_functions` — `mesh_position_local_to_world`, etc.
- `bevy_pbr::pbr_bindings` — PBR material bindings
- `bevy_pbr::pbr_functions` — PBR lighting functions
- `bevy_pbr::pbr_types` — `PbrInput`, `StandardMaterial`
- `bevy_pbr::forward_io` — `VertexOutput`, `FragmentOutput`
- `bevy_pbr::shadow_sampling` — shadow map sampling
- `bevy_pbr::clustered_forward` — cluster lighting
- `bevy_pbr::fog` — fog
- `bevy_pbr::skinning` — skinning

From `bevy_render`:
- `bevy_render::globals` — `Globals { time, delta_time, frame_count }`
- `bevy_render::maths` — PI, helpers
- `bevy_render::color_operations` — tone mapping
- `bevy_render::bindless` — bindless arrays

---

## 8. Render Phases

**Binned phases** (`Opaque3d`): For opaque geometry with batching (GPU multi-draw)
**Sorted phases** (`AlphaMask3d`, `Transparent3d`): Sorted by distance

Create custom phases by implementing `PhaseItem` + `RenderCommand`, registering draw functions, adding render graph nodes.

Standard material draw command chain:
```rust
pub type DrawMaterial = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetMeshBindGroup<2>,
    SetMaterialBindGroup<MATERIAL_BIND_GROUP_INDEX>,
    DrawMesh,
);
```

---

## 9. Bind Groups & Buffers

Binding type constructors (from `bevy::render::render_resource::binding_types`):
- `texture_2d(sample_type)`, `texture_cube(sample_type)`
- `texture_storage_2d(format, access)`
- `sampler(binding_type)`, `comparison_sampler(binding_type)`
- `uniform_buffer::<T>(optional)`, `uniform_buffer_sized(size, optional)`
- `storage_buffer::<T>(read_only)`, `storage_buffer_sized(size, read_only)`
- `binding_array<T>(count)` (bindless)

Buffer types:
- `UniformBuffer<T>` — small CPU-uploaded data, call `write_buffer()` each frame
- `ShaderStorageBuffer` — asset type for GPU storage, `set_data()` at runtime
- `DynamicUniformBuffer<T>` — per-entity uniforms with dynamic offsets
- `GpuShaderStorageBuffer` — render-world GPU representation

---

## 10. Key Examples Index

| File | Concept |
|---|---|
| `shader_material.rs` | Basic custom material |
| `extended_material.rs` | Wrapping StandardMaterial |
| `compute_shader_game_of_life.rs` | Compute pipeline + render graph node |
| `gpu_readback.rs` | Reading GPU data back |
| `custom_post_processing.rs` | Full post-processing pipeline |
| `custom_render_phase.rs` | Custom stencil render phase |
| `specialized_mesh_pipeline.rs` | Full custom pipeline with batching |
| `storage_buffer.rs` | Storage buffers in materials |
| `shader_prepass.rs` | Reading depth/normal prepass |
| `custom_vertex_attribute.rs` | Custom vertex data |

---

## Summary: Which API to Use

| Goal | API |
|---|---|
| Custom shading on meshes | `Material` trait + `AsBindGroup` |
| Extend PBR with extra data | `ExtendedMaterial<StandardMaterial, E>` |
| Full custom rendering pass | `SpecializedMeshPipeline` + phase queue |
| GPU compute (particles, physics) | Compute pipeline + render graph node |
| Screen-space effect | Post-processing `ViewNode` + fullscreen triangle |
| New render pass (e.g. stencil) | Custom `PhaseItem` + `RenderCommand` + graph node |
