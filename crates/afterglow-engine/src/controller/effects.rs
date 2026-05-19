use bevy::prelude::*;

use super::FirstPersonControllerConfig;

#[derive(Component, Clone, Debug, Default, PartialEq, Reflect)]
pub struct FirstPersonEffectStack {
    pub effects: Vec<FirstPersonEffect>,
}

#[derive(Clone, Debug, PartialEq, Reflect)]
pub struct FirstPersonEffect {
    pub remaining_ticks: u16,
    pub elapsed_ticks: u16,
    pub blend_in_ticks: u16,
    pub blend_out_ticks: u16,
    pub speed_multiplier: f32,
    pub gravity_multiplier: f32,
    pub look_multiplier: f32,
    pub jump_multiplier: f32,
}

impl Default for FirstPersonEffect {
    fn default() -> Self {
        Self {
            remaining_ticks: 0,
            elapsed_ticks: 0,
            blend_in_ticks: 0,
            blend_out_ticks: 0,
            speed_multiplier: 1.0,
            gravity_multiplier: 1.0,
            look_multiplier: 1.0,
            jump_multiplier: 1.0,
        }
    }
}

impl FirstPersonEffect {
    pub fn speed_multiplier(multiplier: f32, duration_ticks: u16) -> Self {
        Self {
            speed_multiplier: multiplier,
            remaining_ticks: duration_ticks,
            ..default()
        }
    }

    pub fn gravity_multiplier(multiplier: f32, duration_ticks: u16) -> Self {
        Self {
            gravity_multiplier: multiplier,
            remaining_ticks: duration_ticks,
            ..default()
        }
    }

    pub fn look_multiplier(multiplier: f32, duration_ticks: u16) -> Self {
        Self {
            look_multiplier: multiplier,
            remaining_ticks: duration_ticks,
            ..default()
        }
    }

    pub fn jump_multiplier(multiplier: f32, duration_ticks: u16) -> Self {
        Self {
            jump_multiplier: multiplier,
            remaining_ticks: duration_ticks,
            ..default()
        }
    }

    fn weight(&self) -> f32 {
        if self.remaining_ticks == 0 {
            return 0.0;
        }
        let blend_in = if self.blend_in_ticks == 0 {
            1.0
        } else {
            (self.elapsed_ticks as f32 / self.blend_in_ticks as f32).clamp(0.0, 1.0)
        };
        let blend_out = if self.blend_out_ticks == 0 {
            1.0
        } else {
            (self.remaining_ticks as f32 / self.blend_out_ticks as f32).clamp(0.0, 1.0)
        };
        smoothstep(blend_in.min(blend_out))
    }
}

impl FirstPersonEffectStack {
    pub fn push(&mut self, effect: FirstPersonEffect) {
        if effect.remaining_ticks > 0 {
            self.effects.push(effect);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    pub fn tick_fixed(&mut self) {
        for effect in &mut self.effects {
            effect.elapsed_ticks = effect.elapsed_ticks.saturating_add(1);
            effect.remaining_ticks = effect.remaining_ticks.saturating_sub(1);
        }
        self.effects.retain(|effect| effect.remaining_ticks > 0);
    }

    pub fn effective_config(
        &self,
        base: &FirstPersonControllerConfig,
    ) -> FirstPersonControllerConfig {
        let mut config = base.clone();
        let mut speed = 1.0;
        let mut gravity = 1.0;
        let mut look = 1.0;
        let mut jump = 1.0;
        for effect in &self.effects {
            let weight = effect.weight();
            speed *= weighted_multiplier(effect.speed_multiplier, weight);
            gravity *= weighted_multiplier(effect.gravity_multiplier, weight);
            look *= weighted_multiplier(effect.look_multiplier, weight);
            jump *= weighted_multiplier(effect.jump_multiplier, weight);
        }
        config.ground_speed *= speed;
        config.sprint_speed *= speed;
        config.crouch_speed *= speed;
        config.backward_speed *= speed;
        config.side_speed *= speed;
        config.air_wish_speed *= speed;
        config.ground_accel *= speed;
        config.side_accel *= speed;
        config.gravity *= gravity;
        config.jump_speed *= jump;
        config.look_sensitivity *= look;
        config
    }
}

fn weighted_multiplier(multiplier: f32, weight: f32) -> f32 {
    if !multiplier.is_finite() {
        return 1.0;
    }
    1.0 + (multiplier.max(0.0) - 1.0) * weight
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}
