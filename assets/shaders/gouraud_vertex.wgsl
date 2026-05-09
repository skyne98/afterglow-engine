#define_import_path bevy_pbr::gouraud_vertex

#import bevy_pbr::mesh_functions::{mesh_position_local_to_world, mesh_position_world_to_clip}
#import bevy_pbr::mesh_bindings::mesh
#import bevy_pbr::mesh_view_bindings::{view, lights, clusterable_objects}
#import bevy_pbr::clustered_forward::{fragment_cluster_index, unpack_clusterable_object_index_ranges, get_clusterable_object_id}

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vertex(in: Vertex) -> VertexOutput {
    let world_pos = mesh_position_local_to_world(mesh.model, vec4(in.position, 1.0));
    let world_normal = normalize(mat3x3<f32>(mesh.model) * in.normal);

    let clip_pos = mesh_position_world_to_clip(world_pos);

    // Scene ambient
    var total_light = lights.ambient_color.rgb;

    // Cluster-based point/spot lights
    let view_z = clip_pos.z;
    let is_ortho = view.clip_from_view[3].w == 1.0;
    let cluster_idx = fragment_cluster_index(clip_pos.xy, view_z, is_ortho);
    let ranges = unpack_clusterable_object_index_ranges(cluster_idx);

    for (var i = ranges.first_point_light_index_offset; i < ranges.first_spot_light_index_offset; i++) {
        let light_id = get_clusterable_object_id(i);
        let light = clusterable_objects.data[light_id];
        let light_pos = light.position_radius.xyz;
        let light_color = light.color_inverse_square_range.rgb;
        let light_radius = light.position_radius.w;

        let to_light = light_pos - world_pos.xyz;
        let dist = length(to_light);
        if dist < light_radius {
            let attenuation = 1.0 - (dist * dist) / (light_radius * light_radius);
            let NdotL = max(dot(world_normal, normalize(to_light)), 0.0);
            total_light += light_color * NdotL * attenuation;
        }
    }

    // Directional lights
    for (var i = 0u; i < lights.n_directional_lights; i++) {
        let dir_light = lights.directional_lights[i];
        let NdotL = max(dot(world_normal, dir_light.direction_to_light), 0.0);
        total_light += dir_light.color.rgb * NdotL;
    }

    let output_color = vec4(total_light, 1.0);
    return VertexOutput(clip_pos, output_color, in.uv);
}
