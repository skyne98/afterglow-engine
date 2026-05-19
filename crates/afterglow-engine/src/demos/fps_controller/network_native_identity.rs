use crate::{core::identity::StableEntityId, network::AfterglowLightyearConfig};

pub(super) const FPS_DEMO_PROTOCOL_ID: u64 = 0xA6F0_F050_0000_0001;
pub(super) const FPS_DEMO_PRIVATE_KEY: [u8; 32] = [0xAF; 32];
const FPS_DEMO_NATIVE_PLAYER_NAMESPACE: u128 = 0xA6F0_F050_0000_0000_0000_0000_0000_0000;

pub(crate) fn native_player_id(client_id: u64) -> StableEntityId {
    StableEntityId::from_raw(FPS_DEMO_NATIVE_PLAYER_NAMESPACE | client_id as u128)
}

#[cfg(test)]
pub(crate) fn native_host_player_id() -> StableEntityId {
    const HOST_NAMESPACE: u128 = 0xA6F0_F050_0000_0001_0000_0000_0000_0000;
    StableEntityId::from_raw(HOST_NAMESPACE)
}

#[cfg(test)]
pub(crate) fn native_player_client_id(stable_id: StableEntityId) -> Option<u64> {
    let raw = stable_id.as_raw();
    let namespace = raw & !(u64::MAX as u128);
    (namespace == FPS_DEMO_NATIVE_PLAYER_NAMESPACE).then_some(raw as u64)
}

pub(super) fn new_native_client_id() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(1);
    let id = nanos ^ ((std::process::id() as u64) << 32);
    id.max(1)
}

pub(super) fn protocol_id(config: &AfterglowLightyearConfig) -> u64 {
    if config.protocol_id == 0 {
        FPS_DEMO_PROTOCOL_ID
    } else {
        config.protocol_id
    }
}
