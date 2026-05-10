use afterglow_engine::{
    core::identity::StableEntityId,
    input::{InputAction, PlayerCommand},
    network::{
        DeliveryMode, FaultConfig, MemoryTransport, NetChannel, NetworkPlayerId, NetworkTransport,
        PeerId, TransportEvent,
        authority::{CommandAuthorityResult, CommandRejectReason, ServerCommandBuffer},
        interpolation::{RemoteEntitySample, RemoteInterpolationBuffer},
        prediction::ClientPredictionBuffer,
        reconciliation::{
            AuthoritativeCorrection, AuthoritativeUpdateSource, ClientReconciliationQueue,
        },
        session::{NetworkSession, PlatformIdentity},
    },
};

const ALICE: NetworkPlayerId = NetworkPlayerId(1);
const BOB: NetworkPlayerId = NetworkPlayerId(2);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Vec3f {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3f {
    const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    fn mul(self, value: f32) -> Self {
        Self::new(self.x * value, self.y * value, self.z * value)
    }

    fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    fn distance_squared(self, other: Self) -> f32 {
        self.sub(other).dot(self.sub(other))
    }
}

#[test]
fn duplicate_spell_cast_tick_spawns_only_one_projectile() {
    let mut authority = authority_session();
    let first = submit_spell(&mut authority, PeerId(1), ALICE, 7);
    let duplicate = submit_spell(&mut authority, PeerId(1), ALICE, 7);

    assert_eq!(first, CommandAuthorityResult::Accepted);
    assert_eq!(
        duplicate,
        CommandAuthorityResult::Rejected(CommandRejectReason::DuplicateTick)
    );
}

#[test]
fn spoofed_spell_cast_for_another_player_is_rejected() {
    let mut authority = authority_session();

    assert_eq!(
        submit_spell(&mut authority, PeerId(2), ALICE, 8),
        CommandAuthorityResult::Rejected(CommandRejectReason::PlayerNotOwned)
    );
}

#[test]
fn high_speed_projectile_uses_swept_collision_between_ticks() {
    let previous = Vec3f::new(-10.0, 0.0, 0.0);
    let current = Vec3f::new(10.0, 0.0, 0.0);
    let target = Vec3f::new(0.0, 0.5, 0.0);
    let projectile_radius = 0.1;
    let player_radius = 0.75;

    assert!(
        segment_distance_squared(previous, current, target)
            <= (projectile_radius + player_radius) * (projectile_radius + player_radius)
    );
}

#[test]
fn projectile_owner_does_not_hit_self() {
    let projectile_owner = ALICE;
    let checked_player = ALICE;

    assert!(!can_projectile_hit(projectile_owner, checked_player));
    assert!(can_projectile_hit(projectile_owner, BOB));
}

#[test]
fn packet_loss_does_not_create_duplicate_authoritative_spell_hits() {
    let mut sender = MemoryTransport::new(PeerId(1));
    let mut receiver = MemoryTransport::new(PeerId(2)).with_faults(FaultConfig {
        drop_every: Some(2),
        duplicate_every: Some(3),
        ..Default::default()
    });
    sender.connect_peer(PeerId(2));
    receiver.connect_peer(PeerId(1));

    for id in [1_u8, 2, 3] {
        sender.send(
            PeerId(2),
            NetChannel::Events,
            DeliveryMode::Reliable,
            vec![id],
        );
    }
    MemoryTransport::pump_pair(&mut sender, &mut receiver);

    let mut events = Vec::new();
    receiver.poll_events(&mut events);
    let delivered = events
        .into_iter()
        .filter_map(|event| match event {
            TransportEvent::Packet(packet) => Some(packet.payload[0]),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(delivered, [1, 3, 3]);
    assert_eq!(
        delivered
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        2
    );
}

#[test]
fn out_of_order_authoritative_samples_do_not_break_interpolation() {
    let entity = StableEntityId::from_raw(42);
    let mut smoothing = RemoteInterpolationBuffer::default().with_timing(1, 1);

    smoothing.record(entity, 12, sample(12.0));
    smoothing.record(entity, 10, sample(10.0));

    let smoothed = smoothing.sample_at(entity, 11.0).unwrap();
    assert_eq!(smoothed.fields["pos_x"], 11.0);
}

#[test]
fn rejected_spell_cast_drops_local_prediction_on_correction_tick() {
    let mut prediction = ClientPredictionBuffer::default();
    let mut reconciliation = ClientReconciliationQueue::default();
    prediction.record(PlayerCommand {
        player: ALICE,
        tick: 20,
        actions: vec![InputAction::new("cast_spell")],
        ..Default::default()
    });

    let result = reconciliation.reconcile(
        &mut prediction,
        AuthoritativeCorrection {
            player: ALICE,
            tick: 20,
            source: AuthoritativeUpdateSource::Correction,
        },
    );

    assert!(result.replay_commands.is_empty());
    assert_eq!(prediction.pending_len(ALICE), 0);
}

#[test]
fn extrapolation_resumes_when_late_projectile_snapshot_arrives() {
    let entity = StableEntityId::from_raw(77);
    let mut smoothing = RemoteInterpolationBuffer::default().with_timing(1, 1);
    smoothing.record(entity, 1, sample(1.0));
    smoothing.record(entity, 2, sample(2.0));

    assert!(smoothing.sample_at(entity, 4.0).is_none());

    smoothing.record(entity, 4, sample(4.0));
    let resumed = smoothing.sample_at(entity, 3.0).unwrap();
    assert_eq!(resumed.fields["pos_x"], 3.0);
}

fn authority_session() -> ServerCommandBuffer {
    ServerCommandBuffer::default()
}

fn submit_spell(
    authority: &mut ServerCommandBuffer,
    peer: PeerId,
    player: NetworkPlayerId,
    tick: u32,
) -> CommandAuthorityResult {
    let mut session = NetworkSession::default();
    session.connect_peer(PeerId(1), PlatformIdentity::Local);
    session.connect_peer(PeerId(2), PlatformIdentity::Local);
    assert_eq!(session.add_player(PeerId(1)), Some(ALICE));
    assert_eq!(session.add_player(PeerId(2)), Some(BOB));
    authority.submit(
        peer,
        PlayerCommand {
            player,
            tick,
            actions: vec![InputAction::new("cast_spell")],
            ..Default::default()
        },
        &session,
    )
}

fn can_projectile_hit(owner: NetworkPlayerId, checked: NetworkPlayerId) -> bool {
    owner != checked
}

fn sample(x: f32) -> RemoteEntitySample {
    RemoteEntitySample::default().with_field("pos_x", x)
}

fn segment_distance_squared(a: Vec3f, b: Vec3f, point: Vec3f) -> f32 {
    let segment = b.sub(a);
    let length_squared = segment.dot(segment);
    if length_squared == 0.0 {
        return point.distance_squared(a);
    }
    let t = (point.sub(a).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance_squared(a.add(segment.mul(t)))
}
