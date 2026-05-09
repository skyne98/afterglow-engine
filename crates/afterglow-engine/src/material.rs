use bevy::pbr::Material;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::AsBindGroup;
use bevy_shader::ShaderRef;

#[derive(Asset, AsBindGroup, Clone, TypePath)]
pub struct GouraudMaterial {
    #[uniform(0)]
    pub base_color: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub base_color_texture: Option<Handle<Image>>,
    pub alpha_mode: AlphaMode,
}

impl Material for GouraudMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Path("shaders/gouraud_vertex.wgsl".into())
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path("shaders/gouraud_fragment.wgsl".into())
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }
}
