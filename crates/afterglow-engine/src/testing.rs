use bevy::{asset::AssetPlugin, prelude::*};

use crate::core::AfterglowCorePlugin;

pub fn unit_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AfterglowCorePlugin));
    app
}

pub fn asset_unit_app() -> App {
    let mut app = unit_app();
    app.add_plugins(AssetPlugin::default());
    app
}

#[cfg(feature = "test-support")]
pub mod headless_render {
    use super::*;
    use bevy::{
        image::ImagePlugin,
        render::{
            RenderApp, RenderPlugin,
            render_resource::{Texture, TextureView},
            renderer::{
                RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue,
                WgpuWrapper,
            },
            settings::RenderCreation,
        },
    };
    use std::sync::Arc;

    pub fn app() -> Option<App> {
        let render_creation = real_gpu_render_creation()?;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            RenderPlugin {
                render_creation,
                synchronous_pipeline_compilation: true,
                ..default()
            },
            AfterglowCorePlugin,
        ));
        app.finish();
        app.cleanup();
        Some(app)
    }

    pub fn render_world(app: &App) -> &World {
        app.get_sub_app(RenderApp)
            .expect("headless render app should contain RenderApp")
            .world()
    }

    pub fn render_world_mut(app: &mut App) -> &mut World {
        app.get_sub_app_mut(RenderApp)
            .expect("headless render app should contain RenderApp")
            .world_mut()
    }

    pub struct OffscreenTexture {
        pub texture: Texture,
        pub view: TextureView,
        pub size: UVec2,
        pub format: wgpu::TextureFormat,
    }

    pub fn offscreen_texture(
        render_device: &RenderDevice,
        size: UVec2,
        format: wgpu::TextureFormat,
    ) -> OffscreenTexture {
        let texture = render_device.create_texture(&wgpu::TextureDescriptor {
            label: Some("afterglow_test_offscreen_texture"),
            size: wgpu::Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        OffscreenTexture {
            texture,
            view,
            size,
            format,
        }
    }

    fn real_gpu_render_creation() -> Option<RenderCreation> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());

        let adapter =
            bevy::tasks::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            }))
            .ok()?;

        let adapter_info = adapter.get_info();
        let (device, queue) =
            bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .ok()?;

        Some(RenderCreation::manual(
            RenderDevice::from(device),
            RenderQueue(Arc::new(WgpuWrapper::new(queue))),
            RenderAdapterInfo(WgpuWrapper::new(adapter_info)),
            RenderAdapter(Arc::new(WgpuWrapper::new(adapter))),
            RenderInstance(Arc::new(WgpuWrapper::new(instance))),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_app_registers_core_resources() {
        let _app = unit_app();
    }

    #[test]
    fn asset_unit_app_registers_asset_server() {
        let app = asset_unit_app();
        assert!(app.world().contains_resource::<AssetServer>());
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn headless_render_app_registers_render_sub_app() {
        let Some(app) = headless_render::app() else {
            eprintln!("skipping real GPU headless render test: no compatible adapter");
            return;
        };
        let render_world = headless_render::render_world(&app);
        assert!(render_world.contains_resource::<RenderDevice>());
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn headless_render_app_can_create_offscreen_texture() {
        let Some(app) = headless_render::app() else {
            eprintln!("skipping real GPU offscreen texture test: no compatible adapter");
            return;
        };
        let render_world = headless_render::render_world(&app);
        let render_device = render_world.resource::<RenderDevice>();
        let texture = headless_render::offscreen_texture(
            render_device,
            UVec2::new(4, 4),
            wgpu::TextureFormat::Rgba8Unorm,
        );
        assert_eq!(texture.size, UVec2::new(4, 4));
        assert_eq!(texture.format, wgpu::TextureFormat::Rgba8Unorm);
    }

    #[cfg(feature = "test-support")]
    use bevy::render::renderer::RenderDevice;
}
