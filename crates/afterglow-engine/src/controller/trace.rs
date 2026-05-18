use bevy::prelude::*;

#[derive(Resource, Clone, Debug)]
pub struct FirstPersonControllerTrace {
    pub enabled: bool,
    pub max_frames: usize,
    pub controller_frames: Vec<FirstPersonControllerTraceFrame>,
    pub camera_frames: Vec<FirstPersonCameraTraceFrame>,
}

#[derive(Clone, Debug)]
pub struct FirstPersonControllerTraceFrame {
    pub entity: Entity,
    pub tick: u32,
    pub dt: f32,
    pub command_move: Vec2,
    pub command_look: Vec2,
    pub jump_down: bool,
    pub crouch_pressed: bool,
    pub sprint_down: bool,
    pub start_position: Vec3,
    pub after_step_latch_position: Vec3,
    pub after_input_position: Vec3,
    pub after_stance_position: Vec3,
    pub intended_horizontal_delta: Vec3,
    pub horizontal_pushback: Vec3,
    pub after_horizontal_position: Vec3,
    pub step: FirstPersonStepTrace,
    pub after_step_position: Vec3,
    pub gravity_applied: bool,
    pub vertical_delta: Vec3,
    pub vertical_pushback: Vec3,
    pub after_vertical_position: Vec3,
    pub grounded: bool,
    pub ground_contact_ticks: u8,
    pub climbing: bool,
    pub ground_normal: Vec3,
    pub local_speed: Vec2,
    pub velocity: Vec3,
}

#[derive(Clone, Debug)]
pub struct FirstPersonCameraTraceFrame {
    pub camera: Entity,
    pub target: Entity,
    pub dt: f32,
    pub base_position: Vec3,
    pub bob_offset: Vec3,
    pub final_position: Vec3,
    pub bobbing: bool,
    pub bob_phase: f32,
    pub current_bob_amplitude: Vec2,
    pub landing_bounce: f32,
    pub footstep_emitted: bool,
}

#[derive(Clone, Debug)]
pub struct FirstPersonStepTrace {
    pub ran: bool,
    pub accepted: bool,
    pub reject_reason: FirstPersonStepRejectReason,
    pub ray_count: usize,
    pub rays: Vec<FirstPersonStepRayTrace>,
    pub forward_len: f32,
    pub max_step: f32,
    pub lift: f32,
}

#[derive(Clone, Debug)]
pub struct FirstPersonStepRayTrace {
    pub index: usize,
    pub start: Vec3,
    pub end: Vec3,
    pub hit: bool,
    pub hit_distance: f32,
    pub step_height: f32,
    pub fit_position: Vec3,
    pub reject_reason: FirstPersonStepRejectReason,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FirstPersonStepRejectReason {
    #[default]
    NotRun,
    RateLimited,
    NoHorizontalDelta,
    NoRayHit,
    TooLow,
    TooHigh,
    ShapeBlocked,
    Accepted,
}

impl Default for FirstPersonControllerTrace {
    fn default() -> Self {
        Self {
            enabled: false,
            max_frames: 2048,
            controller_frames: Vec::new(),
            camera_frames: Vec::new(),
        }
    }
}

impl FirstPersonControllerTrace {
    pub fn enabled(max_frames: usize) -> Self {
        Self {
            enabled: true,
            max_frames,
            ..default()
        }
    }

    pub fn clear(&mut self) {
        self.controller_frames.clear();
        self.camera_frames.clear();
    }

    pub fn push_controller(&mut self, frame: FirstPersonControllerTraceFrame) {
        if !self.enabled {
            return;
        }
        self.controller_frames.push(frame);
        trim_to_limit(&mut self.controller_frames, self.max_frames);
    }

    pub fn push_camera(&mut self, frame: FirstPersonCameraTraceFrame) {
        if !self.enabled {
            return;
        }
        self.camera_frames.push(frame);
        trim_to_limit(&mut self.camera_frames, self.max_frames);
    }

    pub fn last_controller_for(&self, entity: Entity) -> Option<&FirstPersonControllerTraceFrame> {
        self.controller_frames
            .iter()
            .rev()
            .find(|frame| frame.entity == entity)
    }
}

impl FirstPersonStepTrace {
    pub fn skipped(reason: FirstPersonStepRejectReason) -> Self {
        Self {
            ran: false,
            accepted: false,
            reject_reason: reason,
            ray_count: 0,
            rays: Vec::new(),
            forward_len: 0.0,
            max_step: 0.0,
            lift: 0.0,
        }
    }

    pub fn running(ray_count: usize, forward_len: f32, max_step: f32) -> Self {
        Self {
            ran: true,
            accepted: false,
            reject_reason: FirstPersonStepRejectReason::NoRayHit,
            ray_count,
            rays: Vec::with_capacity(ray_count),
            forward_len,
            max_step,
            lift: 0.0,
        }
    }

    pub fn accept(&mut self, lift: f32) {
        self.accepted = true;
        self.reject_reason = FirstPersonStepRejectReason::Accepted;
        self.lift = lift;
    }
}

fn trim_to_limit<T>(frames: &mut Vec<T>, max_frames: usize) {
    if max_frames == 0 {
        frames.clear();
        return;
    }
    if frames.len() > max_frames {
        frames.drain(0..frames.len() - max_frames);
    }
}
