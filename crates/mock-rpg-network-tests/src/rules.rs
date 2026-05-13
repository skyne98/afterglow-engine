use afterglow_engine::network::{NetworkPlayerId, authority::CommandRejectReason};

use crate::{Chunk, Player, Vec3i};

pub(crate) fn valid_move(current: Vec3i, target: Vec3i) -> bool {
    current.distance_squared(target) <= 160_i64 * 160
}

pub(crate) fn in_reach(a: Vec3i, b: Vec3i) -> bool {
    a.distance_squared(b) <= 8_i64 * 8
}

pub(crate) fn near(a: Chunk, b: Chunk) -> bool {
    (a.0 - b.0).abs() <= 1 && (a.1 - b.1).abs() <= 1 && (a.2 - b.2).abs() <= 1
}

pub(crate) fn net_player(player: Player) -> NetworkPlayerId {
    NetworkPlayerId(player.0)
}

pub(crate) fn reject_reason(reason: CommandRejectReason) -> &'static str {
    match reason {
        CommandRejectReason::UnknownPlayer => "unknown-player",
        CommandRejectReason::PlayerNotOwned => "player-not-owned",
        CommandRejectReason::DuplicateTick => "duplicate-tick",
    }
}
