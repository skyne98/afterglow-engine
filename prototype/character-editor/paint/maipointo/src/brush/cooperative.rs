//! Cooperative (dab-budgeted) stroke driver — a verbatim port of the demo's
//! `mypaint-brush-cooperative.c` wrapper. It re-implements the dab loop
//! against this crate's engine internals so a host can split a stroke across
//! frames without ever exceeding `dab_budget` dabs per call.
//!
//! The split heuristic (idle/painting time thresholds) mirrors the C exactly;
//! any change here must be mirrored there or the demo A/B will diverge.

use super::exp_decay;
use super::{Brush, BrushStateId};
use crate::helpers::{rand_gauss, smallest_angular_difference};
use crate::settings::SettingId;
use crate::surface::Surface;

const AFTERGLOW_PAINT_UNKNOWN: i32 = 0;
const AFTERGLOW_PAINT_YES: i32 = 1;
const AFTERGLOW_PAINT_NO: i32 = 2;

/// Budgeted-stroke continuation state (the C file's `AfterglowBrushStroke`).
#[derive(Default)]
pub struct StrokeState {
    pub active: bool,
    pub painted: i32,
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
    pub tilt_ascension: f32,
    pub tilt_declination: f32,
    pub tilt_declinationx: f32,
    pub tilt_declinationy: f32,
    pub viewzoom: f32,
    pub viewrotation: f32,
    pub barrel_rotation: f32,
    pub linear: bool,
    pub dtime: f64,
    pub dtime_left: f64,
    pub dabs_moved: f32,
}

impl StrokeState {
    /// `afterglow_brush_stroke_cancel`.
    pub fn cancel(&mut self) {
        *self = Self::default();
    }

    /// `afterglow_brush_stroke_pending`.
    pub fn pending(&self) -> bool {
        self.active
    }
}

fn clamp01(v: f32) -> f32 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

impl Brush {
    fn st_base(&self, id: SettingId) -> f32 {
        self.get_base_value(id)
    }

    /// `afterglow_brush_stroke_start` — returns 2 when the caller must split
    /// the stroke (state advanced past a threshold), 1 when queued, 0 when
    /// the budgeted loop finished, negative on error.
    pub fn cooperative_stroke_start(
        &mut self,
        surface: &mut dyn Surface,
        state: &mut StrokeState,
        mut x: f32,
        mut y: f32,
        mut pressure: f32,
        mut xtilt: f32,
        ytilt: f32,
        mut dtime: f64,
        mut viewzoom: f32,
        mut viewrotation: f32,
        mut barrel_rotation: f32,
        linear: bool,
        dab_budget: i32,
    ) -> i32 {
        let max_dtime = 5.0f32;
        if state.active || dab_budget < 1 {
            return -1;
        }

        let mut tilt_ascension = 0.0f32;
        let mut tilt_declination = 90.0f32;
        let mut tilt_declinationx = 90.0f32;
        let mut tilt_declinationy = 90.0f32;
        if xtilt != 0.0 || ytilt != 0.0 {
            xtilt = clampf(xtilt, -1.0, 1.0);
            let ytilt = clampf(ytilt, -1.0, 1.0);
            tilt_ascension = ((-xtilt).atan2(ytilt)).to_degrees();
            let rad = xtilt.hypot(ytilt);
            tilt_declination = 90.0 - rad * 60.0;
            tilt_declinationx = xtilt * 60.0;
            tilt_declinationy = ytilt * 60.0;
        }

        if pressure <= 0.0 {
            pressure = 0.0;
        }
        if !x.is_finite() || !y.is_finite() || x > 1e10 || y > 1e10 || x < -1e10 || y < -1e10 {
            x = 0.0;
            y = 0.0;
            pressure = 0.0;
            viewzoom = 0.0;
            viewrotation = 0.0;
            barrel_rotation = 0.0;
        }
        if dtime <= 0.0 {
            dtime = 0.0001;
        }

        if dtime > 0.100 && pressure != 0.0 && self.st(BrushStateId::Pressure) == 0.0 {
            self.stroke_to(
                surface, x, y, 0.0, 90.0, 0.0, dtime - 0.0001, viewzoom, viewrotation, 0.0,
                linear,
            );
            dtime = 0.0001;
        }

        if self.skip > 0.001 {
            let dist = (self.skip_last_x - x).hypot(self.skip_last_y - y);
            self.skip_last_x = x;
            self.skip_last_y = y;
            self.skipped_dtime += dtime as f32;
            self.skip -= dist;
            dtime = self.skipped_dtime as f64;
            if self.skip > 0.001 && !(dtime > max_dtime as f64 || self.reset_requested) {
                return 1;
            }
            self.skip = 0.0;
            self.skip_last_x = 0.0;
            self.skip_last_y = 0.0;
            self.skipped_dtime = 0.0;
        }

        if self.get_base_value(SettingId::TrackingNoise) != 0.0 {
            let base_radius = self.get_base_value(SettingId::RadiusLogarithmic).exp();
            let noise = base_radius * self.get_base_value(SettingId::TrackingNoise);
            if noise > 0.001 {
                self.skip = 0.5 * noise;
                self.skip_last_x = x;
                self.skip_last_y = y;
                x += noise * rand_gauss(&mut self.rng);
                y += noise * rand_gauss(&mut self.rng);
            }
        }
        let fac = 1.0
            - exp_decay(
                self.get_base_value(SettingId::SlowTracking),
                100.0 * dtime as f32,
            );
        x = self.st(BrushStateId::X) + (x - self.st(BrushStateId::X)) * fac;
        y = self.st(BrushStateId::Y) + (y - self.st(BrushStateId::Y)) * fac;

        if dtime > max_dtime as f64 || self.reset_requested {
            self.reset_requested = false;
            self.reset();
            self.random_input = self.rng.next();
            self.set_st(BrushStateId::X, x);
            self.set_st(BrushStateId::Y, y);
            self.set_st(BrushStateId::Pressure, pressure);
            self.set_st(BrushStateId::ActualX, x);
            self.set_st(BrushStateId::ActualY, y);
            self.set_st(BrushStateId::Stroke, 1.0);
            return 2;
        }

        state.active = true;
        state.painted = AFTERGLOW_PAINT_UNKNOWN;
        state.x = x;
        state.y = y;
        state.pressure = pressure;
        state.tilt_ascension = tilt_ascension;
        state.tilt_declination = tilt_declination;
        state.tilt_declinationx = tilt_declinationx;
        state.tilt_declinationy = tilt_declinationy;
        state.viewzoom = viewzoom;
        state.viewrotation = viewrotation;
        state.barrel_rotation = barrel_rotation;
        state.linear = linear;
        state.dtime = dtime;
        state.dtime_left = dtime;
        state.dabs_moved = self.st(BrushStateId::PartialDabs);
        self.cooperative_stroke_continue(surface, state, dab_budget)
    }

    /// `afterglow_brush_stroke_continue`.
    pub fn cooperative_stroke_continue(
        &mut self,
        surface: &mut dyn Surface,
        state: &mut StrokeState,
        dab_budget: i32,
    ) -> i32 {
        if !state.active || dab_budget < 1 {
            return -1;
        }
        let mut processed = 0;
        let mut dabs_todo = self.count_dabs_to(state.x, state.y, state.dtime_left as f32);
        if !dabs_todo.is_finite() {
            state.cancel();
            return -2;
        }

        while state.dabs_moved + dabs_todo >= 1.0 && processed < dab_budget {
            let step_ddab;
            if state.dabs_moved > 0.0 {
                step_ddab = 1.0 - state.dabs_moved;
                state.dabs_moved = 0.0;
            } else {
                step_ddab = 1.0;
            }
            let frac = step_ddab / dabs_todo;
            let old_x = self.st(BrushStateId::X);
            let old_y = self.st(BrushStateId::Y);
            let old_pressure = self.st(BrushStateId::Pressure);
            let old_dtime_left = state.dtime_left;
            let step_dx = frac * (state.x - old_x);
            let step_dy = frac * (state.y - old_y);
            let step_dpressure = frac * (state.pressure - old_pressure);
            // C: frac * (dtime_left (double) - 0.0) — double product.
            let step_dtime = (frac as f64 * (state.dtime_left - 0.0)) as f32;
            let step_declination =
                frac * (state.tilt_declination - self.st(BrushStateId::Declination));
            let step_declinationx =
                frac * (state.tilt_declinationx - self.st(BrushStateId::Declinationx));
            let step_declinationy =
                frac * (state.tilt_declinationy - self.st(BrushStateId::Declinationy));
            let step_ascension = frac
                * smallest_angular_difference(
                    self.st(BrushStateId::Ascension),
                    state.tilt_ascension,
                );
            let step_barrel_rotation = frac
                * smallest_angular_difference(
                    self.st(BrushStateId::BarrelRotation),
                    state.barrel_rotation * 360.0,
                );

            self.update_states_and_setting_values(
                step_ddab,
                step_dx,
                step_dy,
                step_dpressure,
                step_declination,
                step_ascension,
                step_dtime,
                state.viewzoom,
                state.viewrotation,
                step_declinationx,
                step_declinationy,
                step_barrel_rotation,
            );

            self.set_st(BrushStateId::Flip, self.st(BrushStateId::Flip) * -1.0);
            let painted_now = self.prepare_and_draw_dab(surface, state.linear);
            if painted_now {
                state.painted = AFTERGLOW_PAINT_YES;
            } else if state.painted == AFTERGLOW_PAINT_UNKNOWN {
                state.painted = AFTERGLOW_PAINT_NO;
            }
            self.random_input = self.rng.next();
            state.dtime_left -= step_dtime as f64;
            processed += 1;

            dabs_todo = self.count_dabs_to(state.x, state.y, state.dtime_left as f32);
            if !dabs_todo.is_finite() {
                state.cancel();
                return -2;
            }
            if state.dabs_moved + dabs_todo >= 1.0
                && self.st(BrushStateId::X) == old_x
                && self.st(BrushStateId::Y) == old_y
                && self.st(BrushStateId::Pressure) == old_pressure
                && state.dtime_left == old_dtime_left
            {
                state.cancel();
                return -2;
            }
        }

        if state.dabs_moved + dabs_todo >= 1.0 {
            return 0;
        }
        self.cooperative_finish_stroke(state)
    }

    /// `finish_stroke` — flushes the remaining fractional dab gap and applies
    /// the stroke-split heuristic.
    fn cooperative_finish_stroke(&mut self, state: &mut StrokeState) -> i32 {
        let dabs_todo = self.count_dabs_to(state.x, state.y, state.dtime_left as f32);
        let step_ddab = dabs_todo;
        let step_dx = state.x - self.st(BrushStateId::X);
        let step_dy = state.y - self.st(BrushStateId::Y);
        let step_dpressure = state.pressure - self.st(BrushStateId::Pressure);
        let step_declination =
            state.tilt_declination - self.st(BrushStateId::Declination);
        let step_declinationx =
            state.tilt_declinationx - self.st(BrushStateId::Declinationx);
        let step_declinationy =
            state.tilt_declinationy - self.st(BrushStateId::Declinationy);
        let step_ascension = smallest_angular_difference(
            self.st(BrushStateId::Ascension),
            state.tilt_ascension,
        );
        let step_dtime = state.dtime_left as f32;
        let step_barrel_rotation = smallest_angular_difference(
            self.st(BrushStateId::BarrelRotation),
            state.barrel_rotation * 360.0,
        );

        self.update_states_and_setting_values(
            step_ddab,
            step_dx,
            step_dy,
            step_dpressure,
            step_declination,
            step_ascension,
            step_dtime,
            state.viewzoom,
            state.viewrotation,
            step_declinationx,
            step_declinationy,
            step_barrel_rotation,
        );
        self.set_st(
            BrushStateId::PartialDabs,
            state.dabs_moved + dabs_todo,
        );

        let mut split = false;
        let mut painted = state.painted;
        if painted == AFTERGLOW_PAINT_UNKNOWN {
            if self.stroke_current_idling_time > 0.0 || self.stroke_total_painting_time == 0.0 {
                painted = AFTERGLOW_PAINT_NO;
            } else {
                painted = AFTERGLOW_PAINT_YES;
            }
        }
        if painted == AFTERGLOW_PAINT_YES {
            self.stroke_total_painting_time += state.dtime;
            self.stroke_current_idling_time = 0.0;
            if self.stroke_total_painting_time > 4.0 + 3.0 * state.pressure as f64
                && step_dpressure >= 0.0
            {
                split = true;
            }
        } else {
            self.stroke_current_idling_time += state.dtime;
            if self.stroke_total_painting_time == 0.0 {
                if self.stroke_current_idling_time > 1.0 {
                    split = true;
                }
            } else if self.stroke_total_painting_time + self.stroke_current_idling_time
                > 0.9 + 5.0 * state.pressure as f64
            {
                split = true;
            }
        }
        state.active = false;
        if split {
            2
        } else {
            1
        }
    }
}
