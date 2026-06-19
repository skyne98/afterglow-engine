use bevy::prelude::*;

use super::{protocol::*, rope::HIGHLIGHT_SWITCH_MARGIN, scene::PlayerName};
use crate::{core::identity::StableEntityId, network::AfterglowNetworkContext};

/// Component marking a box as currently highlighted (nearest to local player).
#[derive(Component)]
pub struct Highlighted;

fn box_has_link(target: StableEntityId, links: &Query<(Entity, &RopeLink)>) -> bool {
    links.iter().any(|(_, link)| link.target == target)
}

/// Highlight the nearest unlinked box to the local player.
pub fn highlight_nearest_box(
    mut commands: Commands,
    player_name: Res<PlayerName>,
    context: Option<Res<AfterglowNetworkContext>>,
    players: Query<(&PlayerBox, &Transform)>,
    boxes: Query<(Entity, &KinematicBox, &StableEntityId, &Transform)>,
    links: Query<(Entity, &RopeLink)>,
    highlighted: Query<Entity, With<Highlighted>>,
) {
    let status = context.as_deref().map(|ctx| ctx.get_connection_status());
    let local_member = status.and_then(|s| s.local_member_owner());

    let player_pos = players
        .iter()
        .find(|(pb, _)| {
            pb.owner == player_name.0 || local_member.as_deref() == Some(pb.owner.as_str())
        })
        .map(|(_, t)| t.translation);

    let current: Vec<Entity> = highlighted.iter().collect();
    let current_entity = current.first().copied();
    let desired = player_pos.and_then(|player_pos| {
        let nearest = boxes
            .iter()
            .filter(|(_, _, stable_id, _)| !box_has_link(**stable_id, &links))
            .min_by(|(_, _, _, a), (_, _, _, b)| {
                a.translation
                    .distance(player_pos)
                    .partial_cmp(&b.translation.distance(player_pos))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .and_then(|(entity, _, _, transform)| {
                let distance = transform.translation.distance(player_pos);
                (distance <= ROPE_GRAB_RANGE).then_some((entity, distance))
            });

        let current_distance = current_entity.and_then(|entity| {
            boxes
                .get(entity)
                .ok()
                .and_then(|(_, _, stable_id, transform)| {
                    (!box_has_link(*stable_id, &links))
                        .then_some(transform.translation.distance(player_pos))
                })
                .filter(|distance| *distance <= ROPE_GRAB_RANGE)
        });

        match (current_entity, current_distance, nearest) {
            (Some(current), Some(current_distance), Some((nearest, nearest_distance)))
                if current != nearest
                    && nearest_distance + HIGHLIGHT_SWITCH_MARGIN < current_distance =>
            {
                Some(nearest)
            }
            (Some(current), Some(_), _) => Some(current),
            (_, _, Some((nearest, _))) => Some(nearest),
            _ => None,
        }
    });

    let current: Vec<Entity> = highlighted.iter().collect();
    for entity in current
        .iter()
        .copied()
        .filter(|entity| Some(*entity) != desired)
    {
        commands.entity(entity).remove::<Highlighted>();
    }
    if let Some(entity) = desired {
        if !current.contains(&entity) {
            commands.entity(entity).insert(Highlighted);
        }
    }
}

/// Updates the material color of highlighted boxes. Local-only.
pub fn update_highlight_colors(
    mut materials: ResMut<Assets<StandardMaterial>>,
    boxes: Query<
        (
            &MeshMaterial3d<StandardMaterial>,
            &super::scene::BoxMaterial,
        ),
        Without<Highlighted>,
    >,
    highlighted: Query<
        (
            &MeshMaterial3d<StandardMaterial>,
            &super::scene::BoxMaterial,
        ),
        With<Highlighted>,
    >,
) {
    for (mat_handle, box_mat) in boxes.iter() {
        if let Some(mat) = materials.get_mut(mat_handle) {
            mat.base_color = Color::hsla(box_mat.base_hue, 0.7, 0.5, 1.0);
            mat.emissive = LinearRgba::BLACK;
        }
    }

    for (mat_handle, box_mat) in highlighted.iter() {
        if let Some(mat) = materials.get_mut(mat_handle) {
            mat.base_color = Color::hsla(box_mat.base_hue, 0.9, 0.7, 1.0);
            let glow = Color::hsla(box_mat.base_hue, 0.9, 0.5, 1.0).to_srgba();
            mat.emissive = LinearRgba::new(glow.red * 0.3, glow.green * 0.3, glow.blue * 0.3, 1.0);
        }
    }
}

/// Draws a line from each rope link to its owning player and target box.
pub fn draw_ropes(
    mut gizmos: Gizmos,
    players: Query<(&PlayerBox, &Transform)>,
    boxes: Query<(&KinematicBox, &StableEntityId, &Transform)>,
    links: Query<&RopeLink>,
) {
    for link in links.iter() {
        let Some((_, player_transform)) =
            players.iter().find(|(pb, _)| pb.owner == link.player_owner)
        else {
            continue;
        };
        let Some((_, _, box_transform)) = boxes.iter().find(|(_, id, _)| **id == link.target)
        else {
            continue;
        };
        gizmos.line(
            player_transform.translation,
            box_transform.translation,
            Color::srgb(0.8, 0.6, 0.2),
        );
    }
}
