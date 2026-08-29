//! The brush engine — port of `mypaint-brush.c` (state machine, settings
//! evaluation, dab preparation, smudge) with the states enum from
//! `mypaint-brush-settings-gen.h`.
//!
//! Every C implicit float↔double promotion is reproduced explicitly:
//! - `expf`/`hypotf`/`atan2f`/`logf`/`powf` are float libm calls,
//! - `exp`/`log`/`hypot`/`atan2`/`floor` (no `f` suffix) are double libm
//!   calls, narrowed to float where the C assigns to float.

use crate::helpers::{
    clamp, hsv_to_rgb_float, hsl_to_rgb_float, mix_colors, mod_arith, rand_gauss,
    rgb_to_hsl_float, rgb_to_hsv_float, smallest_angular_difference, sqr,
};
use crate::mapping::Mapping;
use crate::rngdouble::RngDouble;
use crate::surface::Surface;
use crate::settings::{BrushStateId, InputId, SettingId, BRUSH_STATES_COUNT, INPUTS, SETTINGS};

/// `ACTUAL_RADIUS_MIN`.
pub const ACTUAL_RADIUS_MIN: f32 = 0.2;
/// `ACTUAL_RADIUS_MAX`.
pub const ACTUAL_RADIUS_MAX: f32 = 1000.0;
/// `GRID_SIZE`.
pub const GRID_SIZE: f32 = 256.0;

/// Smudge bucket layout (`SMUDGE_R..SMUDGE_BUCKET_SIZE` in the C). The first
/// bucket is the state slots `SMUDGE_RA..LAST_GETCOLOR_RECENTNESS`.
pub const SMUDGE_BUCKET_SIZE: usize = 9;
const SMUDGE_R: usize = 0;
const PREV_COL_R: usize = 4;
const PREV_COL_RECENTNESS: usize = 8;

/// `DEGREES(x)`: `(x / 2π) * 360` in double precision, narrowed to float.
#[inline]
fn degrees(x: f32) -> f32 {
    ((x as f64) / (2.0 * std::f64::consts::PI) * 360.0) as f32
}

/// `RADIANS(x)`: `x * π / 180` in double precision, narrowed to float.
#[inline]
fn radians(x: f32) -> f32 {
    ((x as f64) * std::f64::consts::PI / 180.0) as f32
}

/// `exp_decay` (double `exp` → float result).
fn exp_decay(t_const: f32, t: f32) -> f32 {
    if t_const <= 0.001 {
        return 0.0;
    }
    let arg: f32 = -t / t_const;
    (arg as f64).exp() as f32
}

/// The brush engine (`MyPaintBrush`).
pub struct Brush {
    stroke_total_painting_time: f64,
    stroke_current_idling_time: f64,

    states: [f32; BRUSH_STATES_COUNT],
    smudge_buckets: Vec<f32>,
    num_buckets: i32,
    min_bucket_used: i32,
    max_bucket_used: i32,

    random_input: f64,
    skip: f32,
    skip_last_x: f32,
    skip_last_y: f32,
    skipped_dtime: f32,
    rng: RngDouble,

    settings: Vec<Mapping>,
    settings_value: Vec<f32>,

    speed_mapping_gamma: [f32; 2],
    speed_mapping_m: [f32; 2],
    speed_mapping_q: [f32; 2],

    reset_requested: bool,
}


impl Brush {
    #[inline]
    fn st(&self, id: BrushStateId) -> f32 {
        self.states[id.index()]
    }

    #[inline]
    fn set_st(&mut self, id: BrushStateId, v: f32) {
        self.states[id.index()] = v;
    }

    /// `mypaint_brush_new` (no buckets).
    pub fn new() -> Self {
        Self::new_with_buckets(0)
    }

    /// `mypaint_brush_new_with_buckets`.
    pub fn new_with_buckets(num_smudge_buckets: i32) -> Self {
        let mut brush = Self {
            stroke_total_painting_time: 0.0,
            stroke_current_idling_time: 0.0,
            states: [0.0; BRUSH_STATES_COUNT],
            smudge_buckets: Vec::new(),
            num_buckets: 0,
            min_bucket_used: -1,
            max_bucket_used: -1,
            random_input: 0.0,
            skip: 0.0,
            skip_last_x: 0.0,
            skip_last_y: 0.0,
            skipped_dtime: 0.0,
            rng: RngDouble::new(1000),
            settings: (0..SETTINGS).map(|_| Mapping::new(INPUTS)).collect(),
            settings_value: vec![0.0; SETTINGS],
            speed_mapping_gamma: [0.0; 2],
            speed_mapping_m: [0.0; 2],
            speed_mapping_q: [0.0; 2],
            reset_requested: false,
        };
        if num_smudge_buckets > 0 {
            brush.smudge_buckets =
                vec![0.0; num_smudge_buckets as usize * SMUDGE_BUCKET_SIZE];
            brush.num_buckets = num_smudge_buckets;
            brush.min_bucket_used = 0;
            brush.max_bucket_used = brush.num_buckets - 1;
        }
        brush.reset();
        brush.new_stroke();
        brush.settings_base_values_have_changed();
        brush.reset_requested = true;
        brush
    }

    /// `brush_reset`.
    pub fn reset(&mut self) {
        self.skip = 0.0;
        self.skip_last_x = 0.0;
        self.skip_last_y = 0.0;
        self.skipped_dtime = 0.0;
        self.states = [0.0; BRUSH_STATES_COUNT];
        // Set the flip state so it starts at "1" for the first dab.
        self.set_st(BrushStateId::Flip, -1.0);
        if !self.smudge_buckets.is_empty() && self.min_bucket_used != -1 {
            let lo = self.min_bucket_used as usize * SMUDGE_BUCKET_SIZE;
            let hi = (self.max_bucket_used as usize + 1) * SMUDGE_BUCKET_SIZE;
            self.smudge_buckets[lo..hi].fill(0.0);
            self.min_bucket_used = -1;
            self.max_bucket_used = -1;
        }
    }

    /// `mypaint_brush_new_stroke`.
    pub fn new_stroke(&mut self) {
        self.stroke_current_idling_time = 0.0;
        self.stroke_total_painting_time = 0.0;
    }

    /// `mypaint_brush_reset` (queued).
    pub fn request_reset(&mut self) {
        self.reset_requested = true;
    }

    /// `mypaint_brush_set_base_value`.
    pub fn set_base_value(&mut self, id: SettingId, value: f32) {
        self.settings[id.index()].set_base_value(value);
        self.settings_base_values_have_changed();
    }

    /// `mypaint_brush_get_base_value`.
    pub fn get_base_value(&self, id: SettingId) -> f32 {
        self.settings[id.index()].get_base_value()
    }

    /// `mypaint_brush_set_mapping_n`.
    pub fn set_mapping_n(&mut self, id: SettingId, input: InputId, n: usize) {
        self.settings[id.index()].set_n(input.index(), n);
    }

    /// `mypaint_brush_set_mapping_point`.
    pub fn set_mapping_point(
        &mut self,
        id: SettingId,
        input: InputId,
        index: usize,
        x: f32,
        y: f32,
    ) {
        self.settings[id.index()].set_point(input.index(), index, x, y);
    }

    /// `mypaint_brush_is_constant`.
    pub fn is_constant(&self, id: SettingId) -> bool {
        self.settings[id.index()].is_constant()
    }

    /// `mypaint_brush_get_state`.
    pub fn get_state(&self, id: BrushStateId) -> f32 {
        self.states[id.index()]
    }

    /// `mypaint_brush_set_state`.
    pub fn set_state(&mut self, id: BrushStateId, value: f32) {
        self.states[id.index()] = value;
    }

    #[inline]
    fn setting(&self, id: SettingId) -> f32 {
        self.settings_value[id.index()]
    }

    /// `settings_base_values_have_changed`.
    pub fn settings_base_values_have_changed(&mut self) {
        for i in 0..2 {
            let base = if i == 0 {
                self.get_base_value(SettingId::Speed1Gamma)
            } else {
                self.get_base_value(SettingId::Speed2Gamma)
            };
            // C: expf (float)
            let gamma: f32 = base.exp();
            let fix1_x = 45.0f32;
            let fix1_y = 0.5f32;
            let fix2_x = 45.0f32;
            let fix2_dy = 0.015f32;

            // C: log(fix1_x + gamma) — double log.
            let c1: f32 = ((fix1_x + gamma) as f64).ln() as f32;
            let m = fix2_dy * (fix2_x + gamma);
            let q = fix1_y - m * c1;

            self.speed_mapping_gamma[i] = gamma;
            self.speed_mapping_m[i] = m;
            self.speed_mapping_q[i] = q;
        }
    }

    /// `mypaint_brush_from_defaults`.
    pub fn from_defaults(&mut self) {
        for s in 0..SETTINGS {
            for i in 0..INPUTS {
                self.settings[s].set_n(i, 0);
            }
            let def = crate::settings::SETTING_INFOS[s].default;
            self.settings[s].set_base_value(def);
        }
        self.set_mapping_n(SettingId::OpaqueMultiply, InputId::Pressure, 2);
        self.set_mapping_point(SettingId::OpaqueMultiply, InputId::Pressure, 0, 0.0, 0.0);
        self.set_mapping_point(SettingId::OpaqueMultiply, InputId::Pressure, 1, 1.0, 1.0);
        self.settings_base_values_have_changed();
    }
}

#[derive(Clone, Copy)]
struct Offsets {
    x: f32,
    y: f32,
}

/// `directional_offsets`. All trig is double-precision (`cos`/`sin`,
/// `atan2f` is the float variant, `fmodf` is float) matching the C.
fn directional_offsets(brush: &Brush, base_radius: f32, brush_flip: i32) -> Offsets {
    let sv = |id: SettingId| brush.settings_value[id.index()];
    let st = |id: BrushStateId| brush.states[id.index()];

    // C: expf (float)
    let offset_mult: f32 = sv(SettingId::OffsetMultiplier).exp();
    if !offset_mult.is_finite() {
        return Offsets { x: 0.0, y: 0.0 };
    }

    let mut dx = sv(SettingId::OffsetX);
    let mut dy = sv(SettingId::OffsetY);

    let offset_angle_adj = sv(SettingId::OffsetAngleAdj);
    let dir_angle_dy = st(BrushStateId::DirectionAngleDy);
    let dir_angle_dx = st(BrushStateId::DirectionAngleDx);
    // C: fmodf(DEGREES(atan2f(dy, dx)) - 90, 360)
    let angle_deg = degrees(atan2f(dir_angle_dy, dir_angle_dx));
    let angle_deg = fmodf_impl(angle_deg - 90.0, 360.0);

    // offset to one side of direction
    let offset_angle = sv(SettingId::OffsetAngle);
    if offset_angle != 0.0 {
        let dir_angle = radians(angle_deg + offset_angle_adj);
        dx += cos(dir_angle) * offset_angle;
        dy += sin(dir_angle) * offset_angle;
    }

    // offset to one side of ascension angle
    let view_rotation = st(BrushStateId::Viewrotation);
    let offset_angle_asc = sv(SettingId::OffsetAngleAsc);
    if offset_angle_asc != 0.0 {
        let ascension = st(BrushStateId::Ascension);
        let asc_angle = radians(ascension - view_rotation + offset_angle_adj);
        dx += cos(asc_angle) * offset_angle_asc;
        dy += sin(asc_angle) * offset_angle_asc;
    }

    // offset to one side of view orientation
    let view_offset = sv(SettingId::OffsetAngleView);
    if view_offset != 0.0 {
        let view_angle = radians(view_rotation + offset_angle_adj);
        dx += cos(-view_angle) * view_offset;
        dy += sin(-view_angle) * view_offset;
    }

    // offset mirrored to sides of direction
    let offset_dir_mirror = max(0.0, sv(SettingId::OffsetAngle2));
    if offset_dir_mirror != 0.0 {
        let dir_mirror_angle = radians(angle_deg + offset_angle_adj * brush_flip as f32);
        let offset_factor = offset_dir_mirror * brush_flip as f32;
        dx += cos(dir_mirror_angle) * offset_factor;
        dy += sin(dir_mirror_angle) * offset_factor;
    }

    // offset mirrored to sides of ascension angle
    let offset_asc_mirror = max(0.0, sv(SettingId::OffsetAngle2Asc));
    if offset_asc_mirror != 0.0 {
        let ascension = st(BrushStateId::Ascension);
        let asc_angle = radians(ascension - view_rotation + offset_angle_adj * brush_flip as f32);
        let offset_factor = brush_flip as f32 * offset_asc_mirror;
        dx += cos(asc_angle) * offset_factor;
        dy += sin(asc_angle) * offset_factor;
    }

    // offset mirrored to sides of view orientation
    let offset_view_mirror = max(0.0, sv(SettingId::OffsetAngle2View));
    if offset_view_mirror != 0.0 {
        let offset_factor = brush_flip as f32 * offset_view_mirror;
        let offset_angle_rad = radians(view_rotation + offset_angle_adj);
        dx += cos(-offset_angle_rad) * offset_factor;
        dy += sin(-offset_angle_rad) * offset_factor;
    }

    // Clamp the final offsets (avoid extreme redraw memory use).
    const LIM: f32 = 3240.0;
    let base_mul = base_radius * offset_mult;
    Offsets {
        x: clamp(dx * base_mul, -LIM, LIM),
        y: clamp(dy * base_mul, -LIM, LIM),
    }
}

#[inline]
fn cos(x: f32) -> f32 {
    (x as f64).cos() as f32
}
#[inline]
fn sin(x: f32) -> f32 {
    (x as f64).sin() as f32
}
#[inline]
fn atan2f(y: f32, x: f32) -> f32 {
    f32::atan2(y, x)
}
#[inline]
fn fmodf_impl(a: f32, n: f32) -> f32 {
    a % n // C fmodf
}

/// C `MAX(a, b)` on floats.
#[inline]
fn max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

#[inline]
fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

impl Brush {
    /// `update_states_and_setting_values` — one brush "simulation" step.
    #[allow(clippy::too_many_arguments)]
    fn update_states_and_setting_values(
        &mut self,
        step_ddab: f32,
        step_dx: f32,
        step_dy: f32,
        step_dpressure: f32,
        step_declination: f32,
        step_ascension: f32,
        step_dtime: f32,
        step_viewzoom: f32,
        step_viewrotation: f32,
        step_declinationx: f32,
        step_declinationy: f32,
        step_barrel_rotation: f32,
    ) {
        let mut step_dtime = step_dtime;
        if step_dtime < 0.0 {
            step_dtime = 0.001;
        } else if step_dtime == 0.0 {
            step_dtime = 0.001;
        }

        self.set_st(BrushStateId::X, self.st(BrushStateId::X) + step_dx);
        self.set_st(BrushStateId::Y, self.st(BrushStateId::Y) + step_dy);
        self.set_st(BrushStateId::Pressure, self.st(BrushStateId::Pressure) + step_dpressure);

        self.set_st(BrushStateId::Declination, self.st(BrushStateId::Declination) + step_declination);
        self.set_st(BrushStateId::Ascension, self.st(BrushStateId::Ascension) + step_ascension);
        self.set_st(BrushStateId::Declinationx, self.st(BrushStateId::Declinationx) + step_declinationx);
        self.set_st(BrushStateId::Declinationy, self.st(BrushStateId::Declinationy) + step_declinationy);

        self.set_st(BrushStateId::Viewzoom, step_viewzoom);
        // C: mod_arith(DEGREES(step_viewrotation) + 180.0, 360.0) - 180.0
        let viewrotation =
            mod_arith(degrees(step_viewrotation) + 180.0, 360.0) - 180.0;
        self.set_st(BrushStateId::Viewrotation, viewrotation);

        { // Gridmap state update
            let x = self.st(BrushStateId::ActualX);
            let y = self.st(BrushStateId::ActualY);
            let scale = self.setting(SettingId::GridmapScale).exp();
            let scale_x = self.setting(SettingId::GridmapScaleX);
            let scale_y = self.setting(SettingId::GridmapScaleY);
            let scaled_size = scale * GRID_SIZE;
            let gx = mod_arith((x * scale_x).abs(), scaled_size) / scaled_size * GRID_SIZE;
            let gy = mod_arith((y * scale_y).abs(), scaled_size) / scaled_size * GRID_SIZE;
            self.set_st(BrushStateId::GridmapX, gx);
            self.set_st(BrushStateId::GridmapY, gy);
            if x < 0.0 {
                self.set_st(BrushStateId::GridmapX, GRID_SIZE - gx);
            }
            if y < 0.0 {
                self.set_st(BrushStateId::GridmapY, GRID_SIZE - gy);
            }
        }

        let base_radius = self.get_base_value(SettingId::RadiusLogarithmic).exp();
        self.set_st(
            BrushStateId::BarrelRotation,
            self.st(BrushStateId::BarrelRotation) + step_barrel_rotation,
        );

        if self.st(BrushStateId::Pressure) <= 0.0 {
            self.set_st(BrushStateId::Pressure, 0.0);
        }
        let pressure = self.st(BrushStateId::Pressure);

        { // start / end stroke (for "stroke" input only)
            let lim = 0.0001;
            let threshold = self.get_base_value(SettingId::StrokeThreshold);
            let started = self.st(BrushStateId::StrokeStarted);
            if started == 0.0 && pressure > threshold + lim {
                self.set_st(BrushStateId::StrokeStarted, 1.0);
                self.set_st(BrushStateId::Stroke, 0.0);
            } else if started != 0.0 && pressure <= threshold * 0.9 + lim {
                self.set_st(BrushStateId::StrokeStarted, 0.0);
            }
        }

        // now follows input handling
        let norm_dx = step_dx / step_dtime * self.st(BrushStateId::Viewzoom);
        let norm_dy = step_dy / step_dtime * self.st(BrushStateId::Viewzoom);

        let norm_speed = f32::hypot(norm_dx, norm_dy);
        let norm_dist = f32::hypot(
            step_dx / step_dtime / base_radius,
            step_dy / step_dtime / base_radius,
        ) * step_dtime;

        let mut inputs = [0.0f32; INPUTS];

        inputs[InputId::Pressure.index()] =
            pressure * self.get_base_value(SettingId::PressureGainLog).exp();

        let m0 = self.speed_mapping_m[0];
        let q0 = self.speed_mapping_q[0];
        let m1 = self.speed_mapping_m[1];
        let q1 = self.speed_mapping_q[1];
        // C: log(gamma + STATE(...)) * m + q — double log, float narrow.
        inputs[InputId::Speed1.index()] = (((self.speed_mapping_gamma[0]
            + self.st(BrushStateId::NormSpeed1Slow)) as f64)
            .ln()
            * m0 as f64
            + q0 as f64) as f32;
        inputs[InputId::Speed2.index()] = (((self.speed_mapping_gamma[1]
            + self.st(BrushStateId::NormSpeed2Slow)) as f64)
            .ln()
            * m1 as f64
            + q1 as f64) as f32;

        inputs[InputId::Random.index()] = self.random_input as f32;
        inputs[InputId::Stroke.index()] = min(self.st(BrushStateId::Stroke), 1.0);

        let dir_angle = atan2f(
            self.st(BrushStateId::DirectionDy),
            self.st(BrushStateId::DirectionDx),
        );
        // C: mod_arith(DEGREES(dir_angle) + viewrotation + 180.0, 180.0)
        inputs[InputId::Direction.index()] =
            mod_arith(degrees(dir_angle) + viewrotation + 180.0, 180.0);
        let dir_angle_360 = atan2f(
            self.st(BrushStateId::DirectionAngleDy),
            self.st(BrushStateId::DirectionAngleDx),
        );
        inputs[InputId::DirectionAngle.index()] =
            fmodf_impl(degrees(dir_angle_360) + viewrotation + 360.0, 360.0);
        inputs[InputId::TiltDeclination.index()] = self.st(BrushStateId::Declination);
        inputs[InputId::TiltAscension.index()] = mod_arith(
            self.st(BrushStateId::Ascension) + viewrotation + 180.0,
            360.0,
        ) - 180.0;
        // C: BASEVAL(RADIUS_LOGARITHMIC) - logf(base_radius / VIEWZOOM) — logf float.
        inputs[InputId::Viewzoom.index()] = self.get_base_value(SettingId::RadiusLogarithmic)
            - f32::ln(base_radius / self.st(BrushStateId::Viewzoom));
        inputs[InputId::AttackAngle.index()] = smallest_angular_difference(
            self.st(BrushStateId::Ascension),
            mod_arith(degrees(dir_angle_360) + 90.0, 360.0),
        );
        inputs[InputId::BrushRadius.index()] = self.get_base_value(SettingId::RadiusLogarithmic);

        inputs[InputId::GridmapX.index()] = clamp(
            self.st(BrushStateId::GridmapX),
            0.0,
            GRID_SIZE,
        );
        inputs[InputId::GridmapY.index()] = clamp(
            self.st(BrushStateId::GridmapY),
            0.0,
            GRID_SIZE,
        );

        inputs[InputId::TiltDeclinationx.index()] = self.st(BrushStateId::Declinationx);
        inputs[InputId::TiltDeclinationy.index()] = self.st(BrushStateId::Declinationy);

        inputs[InputId::Custom.index()] = self.st(BrushStateId::CustomInput);
        inputs[InputId::BarrelRotation.index()] =
            mod_arith(self.st(BrushStateId::BarrelRotation), 360.0);

        for (i, v) in self.settings_value.iter_mut().enumerate() {
            *v = self.settings[i].calculate(&inputs);
        }

        self.set_st(
            BrushStateId::DabsPerBasicRadius,
            self.setting(SettingId::DabsPerBasicRadius),
        );
        self.set_st(
            BrushStateId::DabsPerActualRadius,
            self.setting(SettingId::DabsPerActualRadius),
        );
        self.set_st(
            BrushStateId::DabsPerSecond,
            self.setting(SettingId::DabsPerSecond),
        );

        {
            let fac = 1.0 - exp_decay(self.setting(SettingId::SlowTrackingPerDab), step_ddab);
            self.set_st(
BrushStateId::ActualX,
                self.st(BrushStateId::ActualX)
                    + (self.st(BrushStateId::X) - self.st(BrushStateId::ActualX)) * fac,
            );
            self.set_st(
BrushStateId::ActualY,
                self.st(BrushStateId::ActualY)
                    + (self.st(BrushStateId::Y) - self.st(BrushStateId::ActualY)) * fac,
            );
        }

        { // slow speed
            let fac1 = 1.0 - exp_decay(self.setting(SettingId::Speed1Slowness), step_dtime);
            self.set_st(
BrushStateId::NormSpeed1Slow,
                self.st(BrushStateId::NormSpeed1Slow)
                    + (norm_speed - self.st(BrushStateId::NormSpeed1Slow)) * fac1,
            );
            let fac2 = 1.0 - exp_decay(self.setting(SettingId::Speed2Slowness), step_dtime);
            self.set_st(
BrushStateId::NormSpeed2Slow,
                self.st(BrushStateId::NormSpeed2Slow)
                    + (norm_speed - self.st(BrushStateId::NormSpeed2Slow)) * fac2,
            );
        }

        { // slow speed, but as a vector
            let mut time_constant = (self.setting(SettingId::OffsetBySpeedSlowness) * 0.01).exp() - 1.0;
            if time_constant < 0.002 {
                time_constant = 0.002;
            }
            let fac = 1.0 - exp_decay(time_constant, step_dtime);
            self.set_st(
BrushStateId::NormDxSlow,
                self.st(BrushStateId::NormDxSlow) + (norm_dx - self.st(BrushStateId::NormDxSlow)) * fac,
            );
            self.set_st(
BrushStateId::NormDySlow,
                self.st(BrushStateId::NormDySlow) + (norm_dy - self.st(BrushStateId::NormDySlow)) * fac,
            );
        }

        { // orientation (lowpass on dabtime)
            let dx = step_dx * self.st(BrushStateId::Viewzoom);
            let dy = step_dy * self.st(BrushStateId::Viewzoom);

            let step_in_dabtime = f32::hypot(dx, dy);
            // C: expf(SETTING * 0.5) - 1.0 — float exp.
            let fac = 1.0
                - exp_decay(
                    (self.setting(SettingId::DirectionFilter) * 0.5).exp() - 1.0,
                    step_in_dabtime,
                );

            let dx_old = self.st(BrushStateId::DirectionDx);
            let dy_old = self.st(BrushStateId::DirectionDy);

            self.set_st(
BrushStateId::DirectionAngleDx,
                self.st(BrushStateId::DirectionAngleDx) + (dx - self.st(BrushStateId::DirectionAngleDx)) * fac,
            );
            self.set_st(
BrushStateId::DirectionAngleDy,
                self.st(BrushStateId::DirectionAngleDy) + (dy - self.st(BrushStateId::DirectionAngleDy)) * fac,
            );

            // use the opposite speed vector if closer (no 180° turns)
            let mut dx = dx;
            let mut dy = dy;
            if sqr(dx_old - dx) + sqr(dy_old - dy) > sqr(dx_old - (-dx)) + sqr(dy_old - (-dy)) {
                dx = -dx;
                dy = -dy;
            }
            self.set_st(
BrushStateId::DirectionDx,
                self.st(BrushStateId::DirectionDx) + (dx - self.st(BrushStateId::DirectionDx)) * fac,
            );
            self.set_st(
BrushStateId::DirectionDy,
                self.st(BrushStateId::DirectionDy) + (dy - self.st(BrushStateId::DirectionDy)) * fac,
            );
        }

        { // custom input
            let fac = 1.0 - exp_decay(self.setting(SettingId::CustomInputSlowness), 0.1);
            self.set_st(
BrushStateId::CustomInput,
                self.st(BrushStateId::CustomInput)
                    + (self.setting(SettingId::CustomInput) - self.st(BrushStateId::CustomInput)) * fac,
            );
        }

        { // stroke length
            let frequency = (-self.setting(SettingId::StrokeDurationLogarithmic)).exp();
            let stroke = max(0.0, self.st(BrushStateId::Stroke) + norm_dist * frequency);
            let wrap = 1.0 + max(0.0, self.setting(SettingId::StrokeHoldtime));
            if stroke >= wrap && wrap > 9.9 + 1.0 {
                self.set_st(BrushStateId::Stroke, 1.0);
            } else if stroke >= wrap {
                self.set_st(BrushStateId::Stroke, stroke % wrap);
            } else {
                self.set_st(BrushStateId::Stroke, stroke);
            }
        }

        // calculate final radius
        let radius_log = self.setting(SettingId::RadiusLogarithmic);
        self.set_st(BrushStateId::ActualRadius, radius_log.exp());
        if self.st(BrushStateId::ActualRadius) < ACTUAL_RADIUS_MIN {
            self.set_st(BrushStateId::ActualRadius, ACTUAL_RADIUS_MIN);
        }
        if self.st(BrushStateId::ActualRadius) > ACTUAL_RADIUS_MAX {
            self.set_st(BrushStateId::ActualRadius, ACTUAL_RADIUS_MAX);
        }

        // aspect ratio (affects dab spacing)
        self.set_st(
            BrushStateId::ActualEllipticalDabRatio,
            self.setting(SettingId::EllipticalDabRatio),
        );
        self.set_st(
            BrushStateId::ActualEllipticalDabAngle,
            mod_arith(self.setting(SettingId::EllipticalDabAngle) - viewrotation + 180.0, 180.0) - 180.0,
        );
    }
}

impl Brush {
    #[inline]
    fn bucket_view(&mut self) -> &mut [f32] {
        if self.smudge_buckets.is_empty() {
            let start = BrushStateId::SmudgeRa.index();
            return &mut self.states[start..start + SMUDGE_BUCKET_SIZE];
        }
        let idx = clamp(
            roundf(self.setting(SettingId::SmudgeBucket)),
            0.0,
            (self.num_buckets - 1) as f32,
        ) as usize;
        if self.min_bucket_used == -1 || self.min_bucket_used > idx as i32 {
            self.min_bucket_used = idx as i32;
        }
        if self.max_bucket_used < idx as i32 {
            self.max_bucket_used = idx as i32;
        }
        let start = idx * SMUDGE_BUCKET_SIZE;
        &mut self.smudge_buckets[start..start + SMUDGE_BUCKET_SIZE]
    }

    /// `update_smudge_color` (TRUE = caller returns early).
    #[allow(clippy::too_many_arguments)]
    fn update_smudge_color(
        &mut self,
        surface: &mut dyn Surface,
        bucket: &mut [f32],
        smudge_length: f32,
        px: i32,
        py: i32,
        radius: f32,
        legacy_smudge: bool,
        paint_factor: f32,
    ) -> bool {
        let mut update_factor = max(0.01, smudge_length);
        let smudge_length_log = self.setting(SettingId::SmudgeLengthLog);

        let recentness = bucket[PREV_COL_RECENTNESS] * update_factor;
        bucket[PREV_COL_RECENTNESS] = recentness;

        let margin: f32 = 0.0000000000000001;
        let mut r = 0.0f32;
        let mut g = 0.0f32;
        let mut b = 0.0f32;
        let mut a = 0.0f32;
        if recentness < min(1.0, f32::powf(0.5 * update_factor, smudge_length_log) + margin) {
            if recentness == 0.0 {
                update_factor = 0.0;
            }
            bucket[PREV_COL_RECENTNESS] = 1.0;

            let radius_log = self.setting(SettingId::SmudgeRadiusLog);
            let smudge_radius =
                clamp(radius * radius_log.exp(), ACTUAL_RADIUS_MIN, ACTUAL_RADIUS_MAX);

            let sampled = surface.get_color(
                px as f32,
                py as f32,
                smudge_radius,
                if legacy_smudge { -1.0 } else { paint_factor },
            );
            r = sampled[0];
            g = sampled[1];
            b = sampled[2];
            a = sampled[3];

            let smudge_op_lim = self.setting(SettingId::SmudgeTransparency);
            if (smudge_op_lim > 0.0 && a < smudge_op_lim)
                || (smudge_op_lim < 0.0 && a > -smudge_op_lim)
            {
                return true;
            }
            bucket[PREV_COL_R] = r;
            bucket[PREV_COL_R + 1] = g;
            bucket[PREV_COL_R + 2] = b;
            bucket[PREV_COL_R + 3] = a;
        } else {
            r = bucket[PREV_COL_R];
            g = bucket[PREV_COL_R + 1];
            b = bucket[PREV_COL_R + 2];
            a = bucket[PREV_COL_R + 3];
        }

        if legacy_smudge {
            let fac_old = update_factor;
            let fac_new = (1.0 - update_factor) * a;
            bucket[SMUDGE_R] = fac_old * bucket[SMUDGE_R] + fac_new * r;
            bucket[SMUDGE_R + 1] = fac_old * bucket[SMUDGE_R + 1] + fac_new * g;
            bucket[SMUDGE_R + 2] = fac_old * bucket[SMUDGE_R + 2] + fac_new * b;
            bucket[SMUDGE_R + 3] = clamp(fac_old * bucket[SMUDGE_R + 3] + fac_new, 0.0, 1.0);
        } else if a > crate::helpers::WGM_EPSILON * 10.0 {
            let prev_smudge_color = [
                bucket[SMUDGE_R],
                bucket[SMUDGE_R + 1],
                bucket[SMUDGE_R + 2],
                bucket[SMUDGE_R + 3],
            ];
            let sampled_color = [r, g, b, a];
            let smudge_new =
                mix_colors(&prev_smudge_color, &sampled_color, update_factor, paint_factor);
            bucket[SMUDGE_R] = smudge_new[SMUDGE_R];
            bucket[SMUDGE_R + 1] = smudge_new[SMUDGE_R + 1];
            bucket[SMUDGE_R + 2] = smudge_new[SMUDGE_R + 2];
            bucket[SMUDGE_R + 3] = smudge_new[SMUDGE_R + 3];
        } else {
            // avoid spectral noise at low alpha: fade the smudge alpha
            bucket[SMUDGE_R + 3] = (bucket[SMUDGE_R + 3] + a) / 2.0;
        }
        false
    }

    /// `apply_smudge` — returns `eraser_target_alpha`, writing the blended
    /// color into `color` (h/s/v slots holding r/g/b, as in the C).
    fn apply_smudge(
        &self,
        bucket: &[f32],
        smudge_value: f32,
        legacy_smudge: bool,
        paint_factor: f32,
        color: &mut [f32; 3],
    ) -> f32 {
        let smudge_factor = min(1.0, smudge_value);

        let eraser_target_alpha = clamp(
            (1.0 - smudge_factor) + smudge_factor * bucket[SMUDGE_R + 3],
            0.0,
            1.0,
        );

        if eraser_target_alpha > 0.0 {
            if legacy_smudge {
                let col_factor = 1.0 - smudge_factor;
                color[0] = (smudge_factor * bucket[SMUDGE_R] + col_factor * color[0])
                    / eraser_target_alpha;
                color[1] = (smudge_factor * bucket[SMUDGE_R + 1] + col_factor * color[1])
                    / eraser_target_alpha;
                color[2] = (smudge_factor * bucket[SMUDGE_R + 2] + col_factor * color[2])
                    / eraser_target_alpha;
            } else {
                let smudge_color = [
                    bucket[SMUDGE_R],
                    bucket[SMUDGE_R + 1],
                    bucket[SMUDGE_R + 2],
                    bucket[SMUDGE_R + 3],
                ];
                let brush_color = [color[0], color[1], color[2], 1.0];
                let color_new =
                    mix_colors(&smudge_color, &brush_color, smudge_factor, paint_factor);
                color[0] = color_new[SMUDGE_R];
                color[1] = color_new[SMUDGE_R + 1];
                color[2] = color_new[SMUDGE_R + 2];
            }
        } else {
            color[0] = 1.0;
            color[1] = 0.0;
            color[2] = 0.0;
        }
        eraser_target_alpha
    }
}

#[inline]
fn roundf(x: f32) -> f32 {
    f32::round(x)
}

impl Brush {
    /// `prepare_and_draw_dab` — returns whether the surface was modified.
    fn prepare_and_draw_dab(&mut self, surface: &mut dyn Surface, linear: bool) -> bool {
        let opaque_fac = self.setting(SettingId::OpaqueMultiply);
        let mut opaque = max(0.0, self.setting(SettingId::Opaque));
        opaque = clamp(opaque * opaque_fac, 0.0, 1.0);

        let opaque_linearize = self.get_base_value(SettingId::OpaqueLinearize);

        if opaque_linearize != 0.0 {
            let mut dabs_per_pixel = (self.st(BrushStateId::DabsPerActualRadius)
                + self.st(BrushStateId::DabsPerBasicRadius))
                * 2.0;
            if dabs_per_pixel < 1.0 {
                dabs_per_pixel = 1.0;
            }
            dabs_per_pixel = 1.0 + opaque_linearize * (dabs_per_pixel - 1.0);
            let alpha = opaque;
            let beta = 1.0 - alpha;
            let beta_dab = f32::powf(beta, 1.0 / dabs_per_pixel);
            let alpha_dab = 1.0 - beta_dab;
            opaque = alpha_dab;
        }

        let mut x = self.st(BrushStateId::ActualX);
        let mut y = self.st(BrushStateId::ActualY);

        let base_radius = self.get_base_value(SettingId::RadiusLogarithmic).exp();

        let offs = directional_offsets(self, base_radius, self.st(BrushStateId::Flip) as i32);
        x += offs.x;
        y += offs.y;

        let view_zoom = self.st(BrushStateId::Viewzoom);
        let offset_by_speed = self.setting(SettingId::OffsetBySpeed);
        if offset_by_speed != 0.0 {
            x += self.st(BrushStateId::NormDxSlow) * offset_by_speed * 0.1 / view_zoom;
            y += self.st(BrushStateId::NormDySlow) * offset_by_speed * 0.1 / view_zoom;
        }

        let offset_by_random = self.setting(SettingId::OffsetByRandom);
        if offset_by_random != 0.0 {
            let amp = max(0.0, offset_by_random);
            x += rand_gauss(&mut self.rng) * amp * base_radius;
            y += rand_gauss(&mut self.rng) * amp * base_radius;
        }

        let mut radius = self.st(BrushStateId::ActualRadius);
        let radius_by_random = self.setting(SettingId::RadiusByRandom);
        if radius_by_random != 0.0 {
            let noise = rand_gauss(&mut self.rng) * radius_by_random;
            let radius_log = self.setting(SettingId::RadiusLogarithmic) + noise;
            radius = clamp(radius_log.exp(), ACTUAL_RADIUS_MIN, ACTUAL_RADIUS_MAX);
            let alpha_correction = sqr(self.st(BrushStateId::ActualRadius) / radius);
            if alpha_correction <= 1.0 {
                opaque *= alpha_correction;
            }
        }

        let paint_factor = self.setting(SettingId::PaintMode);
        let paint_setting_constant = self.is_constant(SettingId::PaintMode);
        let legacy_smudge = paint_factor <= 0.0 && paint_setting_constant;

        // color part (h/s/v slots hold r/g/b from here)
        let mut color = [
            self.get_base_value(SettingId::ColorH),
            self.get_base_value(SettingId::ColorS),
            self.get_base_value(SettingId::ColorV),
        ];
        hsv_to_rgb_float(&mut color);

        // update smudge color
        let smudge_length = self.setting(SettingId::SmudgeLength);
        if smudge_length < 1.0
            && (self.setting(SettingId::Smudge) != 0.0 || !self.is_constant(SettingId::Smudge))
        {
            // Take a copy of the bucket, update it through the sampler,
            // then write it back (avoids aliasing `self`).
            let mut bucket = std::mem::take(&mut self.smudge_buckets);
            let was_empty = bucket.is_empty();
            if was_empty {
                bucket = self.states[BrushStateId::SmudgeRa.index()
                    ..BrushStateId::SmudgeRa.index() + SMUDGE_BUCKET_SIZE]
                    .to_vec();
            }
            let return_early = self.update_smudge_color(
                surface,
                &mut bucket,
                smudge_length,
                round_i32(x),
                round_i32(y),
                radius,
                legacy_smudge,
                paint_factor,
            );
            if was_empty {
                let start = BrushStateId::SmudgeRa.index();
                self.states[start..start + SMUDGE_BUCKET_SIZE].copy_from_slice(&bucket);
            } else {
                self.smudge_buckets = bucket;
            }
            if return_early {
                return false;
            }
        }

        let mut eraser_target_alpha = 1.0f32;
        let smudge_value = self.setting(SettingId::Smudge);

        if smudge_value > 0.0 {
            let bucket_copy: Vec<f32> = if self.smudge_buckets.is_empty() {
                let start = BrushStateId::SmudgeRa.index();
                self.states[start..start + SMUDGE_BUCKET_SIZE].to_vec()
            } else {
                self.smudge_buckets.clone()
            };
            eraser_target_alpha = self.apply_smudge(
                &bucket_copy,
                smudge_value,
                legacy_smudge,
                paint_factor,
                &mut color,
            );
        }

        if self.setting(SettingId::Eraser) != 0.0 {
            eraser_target_alpha *= 1.0 - self.setting(SettingId::Eraser);
        }

        let using_hsv_dynamics = self.setting(SettingId::ChangeColorH) != 0.0
            || self.setting(SettingId::ChangeColorHsvS) != 0.0
            || self.setting(SettingId::ChangeColorV) != 0.0;
        let using_hsl_dynamics = self.setting(SettingId::ChangeColorL) != 0.0
            || self.setting(SettingId::ChangeColorHslS) != 0.0;
        let using_color_dynamics = using_hsv_dynamics || using_hsl_dynamics;

        if linear && using_color_dynamics {
            color[0] = f32::powf(color[0], 1.0 / 2.2);
            color[1] = f32::powf(color[1], 1.0 / 2.2);
            color[2] = f32::powf(color[2], 1.0 / 2.2);
        }

        if using_hsv_dynamics {
            rgb_to_hsv_float(&mut color);
            color[0] += self.setting(SettingId::ChangeColorH);
            color[1] += color[1] * color[2] * self.setting(SettingId::ChangeColorHsvS);
            color[2] += self.setting(SettingId::ChangeColorV);
            hsv_to_rgb_float(&mut color);
        }

        if using_hsl_dynamics {
            rgb_to_hsl_float(&mut color);
            color[2] += self.setting(SettingId::ChangeColorL);
            color[1] += color[1]
                * min((1.0 - color[2]).abs(), color[2].abs())
                * 2.0
                * self.setting(SettingId::ChangeColorHslS);
            hsl_to_rgb_float(&mut color);
        }

        if linear && using_color_dynamics {
            color[0] = f32::powf(color[0], 2.2);
            color[1] = f32::powf(color[1], 2.2);
            color[2] = f32::powf(color[2], 2.2);
        }

        let mut hardness = clamp(self.setting(SettingId::Hardness), 0.0, 1.0);
        let softness = clamp(self.setting(SettingId::Softness), 0.0, 1.0);

        // anti-aliasing attempt (works well for ink brushes)
        let current_fadeout_in_pixels = radius * (1.0 - hardness);
        let min_fadeout_in_pixels = self.setting(SettingId::AntiAliasing);
        if current_fadeout_in_pixels < min_fadeout_in_pixels {
            let current_optical_radius = radius - (1.0 - hardness) * radius / 2.0;
            let hardness_new = (current_optical_radius - (min_fadeout_in_pixels / 2.0))
                / (current_optical_radius + (min_fadeout_in_pixels / 2.0));
            let radius_new = min_fadeout_in_pixels / (1.0 - hardness_new);
            hardness = hardness_new;
            radius = radius_new;
        }

        // snap to pixel
        let snap_to_pixel = self.setting(SettingId::SnapToPixel);
        if snap_to_pixel > 0.0 {
            let snapped_x = x.floor() + 0.5;
            let snapped_y = y.floor() + 0.5;
            x = x + (snapped_x - x) * snap_to_pixel;
            y = y + (snapped_y - y) * snap_to_pixel;

            let mut snapped_radius = (radius * 2.0).round_ties_even() / 2.0;
            if snapped_radius < 0.5 {
                snapped_radius = 0.5;
            }
            if snap_to_pixel > 0.9999 {
                snapped_radius -= 0.0001;
            }
            radius = radius + (snapped_radius - radius) * snap_to_pixel;
        }

        let dab_ratio = self.st(BrushStateId::ActualEllipticalDabRatio);
        let dab_angle = self.st(BrushStateId::ActualEllipticalDabAngle);
        let lock_alpha = self.setting(SettingId::LockAlpha);
        let colorize = self.setting(SettingId::Colorize);
        let posterize = self.setting(SettingId::Posterize);
        let posterize_num = self.setting(SettingId::PosterizeNum);

        surface.surface_draw_dab(
            x, y, radius, color[0], color[1], color[2], opaque, hardness, softness,
            eraser_target_alpha, dab_ratio, dab_angle, lock_alpha, colorize, posterize,
            posterize_num, paint_factor,
        )
    }

    fn smudge_bucket_ref(&mut self) -> &mut [f32] {
        self.bucket_view()
    }

    /// `count_dabs_to`.
    fn count_dabs_to(&mut self, x: f32, y: f32, dt: f32) -> f32 {
        let base_radius_log = self.get_base_value(SettingId::RadiusLogarithmic);
        let base_radius = clamp(base_radius_log.exp(), ACTUAL_RADIUS_MIN, ACTUAL_RADIUS_MAX);

        if self.st(BrushStateId::ActualRadius) == 0.0 {
            self.set_st(BrushStateId::ActualRadius, base_radius);
        }

        let dx = x - self.st(BrushStateId::X);
        let dy = y - self.st(BrushStateId::Y);

        let dist;
        if self.st(BrushStateId::ActualEllipticalDabRatio) > 1.0 {
            let angle_rad = radians(self.st(BrushStateId::ActualEllipticalDabAngle));
            // C: cos/sin — double functions narrowed to float.
            let cs = cos(angle_rad);
            let sn = sin(angle_rad);
            let yyr = (dy * cs - dx * sn) * self.st(BrushStateId::ActualEllipticalDabRatio);
            let xxr = dy * sn + dx * cs;
            dist = (yyr * yyr + xxr * xxr).sqrt();
        } else {
            dist = f32::hypot(dx, dy);
        }

        let res1 = dist / self.st(BrushStateId::ActualRadius)
            * self.st(BrushStateId::DabsPerActualRadius);
        let res2 = dist / base_radius * self.st(BrushStateId::DabsPerBasicRadius);
        let res3 = dt * self.st(BrushStateId::DabsPerSecond);
        let res4 = res1 + res2 + res3;
        if res4.is_nan() || res4 < 0.0 {
            0.0
        } else {
            res4
        }
    }
}

#[inline]
fn round_i32(x: f32) -> i32 {
    (x + 0.5) as i32
}

impl Brush {
    /// `mypaint_brush_stroke_to — one motion event. Returns non-zero when
    /// the stroke is finished or empty (stroke-split logic).
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_to(
        &mut self,
        surface: &mut dyn Surface,
        x: f32,
        y: f32,
        mut pressure: f32,
        xtilt: f32,
        ytilt: f32,
        dtime: f64,
        viewzoom: f32,
        viewrotation: f32,
        barrel_rotation: f32,
        linear: bool,
    ) -> bool {
        const MAX_DTIME: f64 = 5.0;
        let max_dtime = MAX_DTIME;

        let mut tilt_ascension = 0.0f32;
        let mut tilt_declination = 90.0f32;
        let mut tilt_declinationx = 90.0f32;
        let mut tilt_declinationy = 90.0f32;
        if xtilt != 0.0 || ytilt != 0.0 {
            let xtilt = clamp(xtilt, -1.0, 1.0);
            let ytilt = clamp(ytilt, -1.0, 1.0);
            // C: DEGREES(atan2(-xtilt, ytilt)) — double atan2.
            tilt_ascension = degrees(f64::atan2(-xtilt as f64, ytilt as f64) as f32);
            let rad = f64::hypot(xtilt as f64, ytilt as f64) as f32;
            tilt_declination = 90.0 - (rad * 60.0);
            tilt_declinationx = xtilt * 60.0;
            tilt_declinationy = ytilt * 60.0;
        }

        if pressure <= 0.0 {
            pressure = 0.0;
        }
        let mut x = x;
        let mut y = y;
        if !x.is_finite() || !y.is_finite() || x > 1e10 || y > 1e10 || x < -1e10 || y < -1e10 {
            x = 0.0;
            y = 0.0;
            pressure = 0.0;
        }
        let mut dtime = dtime;
        if dtime < 0.0 {
            dtime = 0.0001;
        }
        if dtime <= 0.0 {
            dtime = 0.0001;
        }

        if dtime > 0.100 && pressure != 0.0 && self.st(BrushStateId::Pressure) == 0.0 {
            // Tablets that skip motion events at zero pressure.
            let _ = self.stroke_to(
                surface,
                x,
                y,
                0.0,
                90.0,
                0.0,
                dtime - 0.0001,
                viewzoom,
                viewrotation,
                0.0,
                linear,
            );
            dtime = 0.0001;
        }

        // skip some length of input if requested (stable tracking noise)
        if self.skip > 0.001 {
            let dist = f32::hypot(self.skip_last_x - x, self.skip_last_y - y);
            self.skip_last_x = x;
            self.skip_last_y = y;
            self.skipped_dtime += dtime as f32;
            self.skip -= dist;
            dtime = self.skipped_dtime as f64;

            if self.skip > 0.001 && !(dtime > max_dtime || self.reset_requested) {
                return false;
            }
            self.skip = 0.0;
            self.skip_last_x = 0.0;
            self.skip_last_y = 0.0;
            self.skipped_dtime = 0.0;
        }

        { // virtual cursor position: tracking noise + slow tracking
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

            let fac = 1.0 - exp_decay(self.get_base_value(SettingId::SlowTracking), 100.0 * dtime as f32);
            x = self.st(BrushStateId::X) + (x - self.st(BrushStateId::X)) * fac;
            y = self.st(BrushStateId::Y) + (y - self.st(BrushStateId::Y)) * fac;
        }

        if dtime > max_dtime || self.reset_requested {
            self.reset_requested = false;
            self.reset();
            self.random_input = self.rng.next();
            self.set_st(BrushStateId::X, x);
            self.set_st(BrushStateId::Y, y);
            self.set_st(BrushStateId::Pressure, pressure);
            self.set_st(BrushStateId::ActualX, x);
            self.set_st(BrushStateId::ActualY, y);
            self.set_st(BrushStateId::Stroke, 1.0);
            return true;
        }

        #[derive(PartialEq)]
        enum Painted {
            Unknown,
            Yes,
            No,
        }
        let mut painted = Painted::Unknown;
        let mut dtime_left = dtime;

        let mut dabs_moved = self.st(BrushStateId::PartialDabs);
        let mut dabs_todo = self.count_dabs_to(x, y, dtime as f32);

        let mut last_step_dpressure = 0.0f32;

        while dabs_moved + dabs_todo >= 1.0 {
            let step_ddab;
            if dabs_moved > 0.0 {
                step_ddab = 1.0 - dabs_moved;
                dabs_moved = 0.0;
            } else {
                step_ddab = 1.0;
            }
            let frac = step_ddab / dabs_todo;
            let step_dx = frac * (x - self.st(BrushStateId::X));
            let step_dy = frac * (y - self.st(BrushStateId::Y));
            let step_dpressure = frac * (pressure - self.st(BrushStateId::Pressure));
            let step_dtime = frac * dtime_left as f32;
            let step_declination = frac * (tilt_declination - self.st(BrushStateId::Declination));
            let step_declinationx = frac * (tilt_declinationx - self.st(BrushStateId::Declinationx));
            let step_declinationy = frac * (tilt_declinationy - self.st(BrushStateId::Declinationy));
            let step_ascension = frac
                * smallest_angular_difference(self.st(BrushStateId::Ascension), tilt_ascension);
            let step_barrel_rotation = frac
                * smallest_angular_difference(
                    self.st(BrushStateId::BarrelRotation),
                    barrel_rotation * 360.0,
                );

            self.update_states_and_setting_values(
                step_ddab,
                step_dx,
                step_dy,
                step_dpressure,
                step_declination,
                step_ascension,
                step_dtime,
                viewzoom,
                viewrotation,
                step_declinationx,
                step_declinationy,
                step_barrel_rotation,
            );

            self.set_st(BrushStateId::Flip, self.st(BrushStateId::Flip) * -1.0);
            let painted_now = self.prepare_and_draw_dab(surface, linear);
            if painted_now {
                painted = Painted::Yes;
            } else if painted == Painted::Unknown {
                painted = Painted::No;
            }

            self.random_input = self.rng.next();

            last_step_dpressure = step_dpressure;
            dtime_left -= step_dtime as f64;
            dabs_todo = self.count_dabs_to(x, y, dtime_left as f32);
        }

        {
            let step_ddab = dabs_todo;
            let step_dx = x - self.st(BrushStateId::X);
            let step_dy = y - self.st(BrushStateId::Y);
            let step_dpressure = pressure - self.st(BrushStateId::Pressure);
            let step_declination = tilt_declination - self.st(BrushStateId::Declination);
            let step_declinationx = tilt_declinationx - self.st(BrushStateId::Declinationx);
            let step_declinationy = tilt_declinationy - self.st(BrushStateId::Declinationy);
            let step_ascension =
                smallest_angular_difference(self.st(BrushStateId::Ascension), tilt_ascension);
            let step_dtime = dtime_left as f32;
            let step_barrel_rotation = smallest_angular_difference(
                self.st(BrushStateId::BarrelRotation),
                barrel_rotation * 360.0,
            );

            self.update_states_and_setting_values(
                step_ddab,
                step_dx,
                step_dy,
                step_dpressure,
                step_declination,
                step_ascension,
                step_dtime,
                viewzoom,
                viewrotation,
                step_declinationx,
                step_declinationy,
                step_barrel_rotation,
            );
        }

        self.set_st(BrushStateId::PartialDabs, dabs_moved + dabs_todo);

        // stroke separation logic (for undo/redo)
        if painted == Painted::Unknown {
            if self.stroke_current_idling_time > 0.0 || self.stroke_total_painting_time == 0.0 {
                painted = Painted::No;
            } else {
                painted = Painted::Yes;
            }
        }
        if painted == Painted::Yes {
            self.stroke_total_painting_time += dtime;
            self.stroke_current_idling_time = 0.0;
            if self.stroke_total_painting_time > 4.0 + 3.0 * pressure as f64 {
                if last_step_dpressure >= 0.0 {
                    return true;
                }
            }
        } else if painted == Painted::No {
            self.stroke_current_idling_time += dtime;
            if self.stroke_total_painting_time == 0.0 {
                if self.stroke_current_idling_time > 1.0 {
                    return true;
                }
            } else if self.stroke_total_painting_time + self.stroke_current_idling_time
                > 0.9 + 5.0 * pressure as f64
            {
                return true;
            }
        }
        false
    }
}
