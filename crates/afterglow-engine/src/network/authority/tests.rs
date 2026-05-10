use super::*;
use crate::{
    input::{InputAction, PlayerCommand},
    network::session::PlatformIdentity,
};

fn session_with_player(peer: PeerId, player: NetworkPlayerId) -> NetworkSession {
    let mut session = NetworkSession::default();
    assert!(session.connect_peer(peer, PlatformIdentity::Local));
    assert_eq!(session.add_player(peer), Some(player));
    session
}

fn command(player: NetworkPlayerId, tick: u32) -> PlayerCommand {
    PlayerCommand {
        player,
        tick,
        actions: vec![InputAction::new("use")],
        ..Default::default()
    }
}

#[test]
fn accepts_commands_from_owning_peer_in_order() {
    let peer = PeerId(7);
    let player = NetworkPlayerId(1);
    let session = session_with_player(peer, player);
    let mut buffer = ServerCommandBuffer::default();

    buffer.submit_many(peer, [command(player, 10), command(player, 11)], &session);

    assert_eq!(
        buffer
            .accepted()
            .iter()
            .map(|accepted| accepted.command.tick)
            .collect::<Vec<_>>(),
        [10, 11]
    );
    assert!(buffer.rejected().is_empty());
}

#[test]
fn rejects_unknown_or_peer_spoofed_players() {
    let owning_peer = PeerId(1);
    let wrong_peer = PeerId(2);
    let player = NetworkPlayerId(1);
    let session = session_with_player(owning_peer, player);
    let mut buffer = ServerCommandBuffer::default();

    assert_eq!(
        buffer.submit(wrong_peer, command(player, 1), &session),
        CommandAuthorityResult::Rejected(CommandRejectReason::PlayerNotOwned)
    );
    assert_eq!(
        buffer.submit(owning_peer, command(NetworkPlayerId(999), 1), &session),
        CommandAuthorityResult::Rejected(CommandRejectReason::UnknownPlayer)
    );
    assert!(buffer.accepted().is_empty());
    assert_eq!(
        buffer
            .rejected()
            .iter()
            .map(|rejected| rejected.reason)
            .collect::<Vec<_>>(),
        [
            CommandRejectReason::PlayerNotOwned,
            CommandRejectReason::UnknownPlayer
        ]
    );
}

#[test]
fn duplicate_ticks_are_rejected_per_player() {
    let peer = PeerId(1);
    let player = NetworkPlayerId(1);
    let session = session_with_player(peer, player);
    let mut buffer = ServerCommandBuffer::default();

    assert_eq!(
        buffer.submit(peer, command(player, 3), &session),
        CommandAuthorityResult::Accepted
    );
    assert_eq!(
        buffer.submit(peer, command(player, 3), &session),
        CommandAuthorityResult::Rejected(CommandRejectReason::DuplicateTick)
    );

    assert_eq!(buffer.accepted().len(), 1);
    assert_eq!(
        buffer.rejected()[0].reason,
        CommandRejectReason::DuplicateTick
    );
}

#[test]
fn begin_frame_clears_outputs_but_keeps_tick_deduplication() {
    let peer = PeerId(1);
    let player = NetworkPlayerId(1);
    let session = session_with_player(peer, player);
    let mut buffer = ServerCommandBuffer::default();

    buffer.submit(peer, command(player, 5), &session);
    buffer.begin_frame();

    assert!(buffer.accepted().is_empty());
    assert!(buffer.rejected().is_empty());
    assert_eq!(
        buffer.submit(peer, command(player, 5), &session),
        CommandAuthorityResult::Rejected(CommandRejectReason::DuplicateTick)
    );
}

#[test]
fn forgetting_player_drops_deduplication_state() {
    let peer = PeerId(1);
    let player = NetworkPlayerId(1);
    let session = session_with_player(peer, player);
    let mut buffer = ServerCommandBuffer::default();

    buffer.submit(peer, command(player, 8), &session);
    buffer.forget_player(player);

    assert_eq!(
        buffer.submit(peer, command(player, 8), &session),
        CommandAuthorityResult::Accepted
    );
}
