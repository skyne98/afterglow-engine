use crate::{
    input::{InputAction, InputAxis, InputAxisValue, PlayerCommand, PointerDevice, PointerInput},
    network::{NetworkPlayerId, ProtocolVersion},
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandDecodeError {
    InvalidJson(String),
    ProtocolMismatch {
        expected: ProtocolVersion,
        got: ProtocolVersion,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub protocol: WireProtocolVersion,
    pub commands: Vec<WirePlayerCommand>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WireProtocolVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WirePlayerCommand {
    pub player: u64,
    pub tick: u32,
    pub axes: Vec<WireAxisValue>,
    pub actions: Vec<String>,
    pub pointers: Vec<WirePointerInput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WireAxisValue {
    pub axis: String,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WirePointerInput {
    pub device: WirePointerDevice,
    pub id: u64,
    pub position: [f32; 2],
    pub delta: [f32; 2],
    pub pressure: Option<f32>,
    pub tilt: Option<[f32; 2]>,
    pub twist: Option<f32>,
    pub primary: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum WirePointerDevice {
    Mouse,
    Touch,
    Pen,
    Unknown,
}

pub fn encode_player_commands(commands: &[PlayerCommand]) -> Result<Vec<u8>, serde_json::Error> {
    let envelope = CommandEnvelope {
        protocol: ProtocolVersion::CURRENT.into(),
        commands: commands.iter().map(WirePlayerCommand::from).collect(),
    };
    serde_json::to_vec(&envelope)
}

pub fn decode_player_commands(bytes: &[u8]) -> Result<Vec<PlayerCommand>, CommandDecodeError> {
    let envelope = serde_json::from_slice::<CommandEnvelope>(bytes)
        .map_err(|err| CommandDecodeError::InvalidJson(err.to_string()))?;
    let got = ProtocolVersion::from(envelope.protocol);
    if got != ProtocolVersion::CURRENT {
        return Err(CommandDecodeError::ProtocolMismatch {
            expected: ProtocolVersion::CURRENT,
            got,
        });
    }
    Ok(envelope
        .commands
        .into_iter()
        .map(PlayerCommand::from)
        .collect())
}

impl From<ProtocolVersion> for WireProtocolVersion {
    fn from(value: ProtocolVersion) -> Self {
        Self {
            major: value.major,
            minor: value.minor,
            patch: value.patch,
        }
    }
}

impl From<WireProtocolVersion> for ProtocolVersion {
    fn from(value: WireProtocolVersion) -> Self {
        Self {
            major: value.major,
            minor: value.minor,
            patch: value.patch,
        }
    }
}

impl From<&PlayerCommand> for WirePlayerCommand {
    fn from(value: &PlayerCommand) -> Self {
        Self {
            player: value.player.0,
            tick: value.tick,
            axes: value.axes.iter().map(WireAxisValue::from).collect(),
            actions: value
                .actions
                .iter()
                .map(|action| action.0.clone())
                .collect(),
            pointers: value.pointers.iter().map(WirePointerInput::from).collect(),
        }
    }
}

impl From<WirePlayerCommand> for PlayerCommand {
    fn from(value: WirePlayerCommand) -> Self {
        Self {
            player: NetworkPlayerId(value.player),
            tick: value.tick,
            axes: value.axes.into_iter().map(InputAxisValue::from).collect(),
            actions: value.actions.into_iter().map(InputAction).collect(),
            pointers: value.pointers.into_iter().map(PointerInput::from).collect(),
        }
    }
}

impl From<&InputAxisValue> for WireAxisValue {
    fn from(value: &InputAxisValue) -> Self {
        Self {
            axis: value.axis.0.clone(),
            value: value.value,
        }
    }
}

impl From<WireAxisValue> for InputAxisValue {
    fn from(value: WireAxisValue) -> Self {
        Self {
            axis: InputAxis(value.axis),
            value: value.value,
        }
    }
}

impl From<&PointerInput> for WirePointerInput {
    fn from(value: &PointerInput) -> Self {
        Self {
            device: value.device.into(),
            id: value.id,
            position: [value.position.x, value.position.y],
            delta: [value.delta.x, value.delta.y],
            pressure: value.pressure,
            tilt: value.tilt.map(|tilt| [tilt.x, tilt.y]),
            twist: value.twist,
            primary: value.primary,
        }
    }
}

impl From<WirePointerInput> for PointerInput {
    fn from(value: WirePointerInput) -> Self {
        Self {
            device: value.device.into(),
            id: value.id,
            position: Vec2::new(value.position[0], value.position[1]),
            delta: Vec2::new(value.delta[0], value.delta[1]),
            pressure: value.pressure,
            tilt: value.tilt.map(|tilt| Vec2::new(tilt[0], tilt[1])),
            twist: value.twist,
            primary: value.primary,
        }
    }
}

impl From<PointerDevice> for WirePointerDevice {
    fn from(value: PointerDevice) -> Self {
        match value {
            PointerDevice::Mouse => Self::Mouse,
            PointerDevice::Touch => Self::Touch,
            PointerDevice::Pen => Self::Pen,
            PointerDevice::Unknown => Self::Unknown,
        }
    }
}

impl From<WirePointerDevice> for PointerDevice {
    fn from(value: WirePointerDevice) -> Self {
        match value {
            WirePointerDevice::Mouse => Self::Mouse,
            WirePointerDevice::Touch => Self::Touch,
            WirePointerDevice::Pen => Self::Pen,
            WirePointerDevice::Unknown => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests;
