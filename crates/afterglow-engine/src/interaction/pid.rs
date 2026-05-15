use bevy::prelude::*;

#[derive(Clone, Debug)]
pub struct PidController {
    pub p: f32,
    pub i: f32,
    pub d: f32,
    errors: Vec<f32>,
    time_steps: Vec<f32>,
    error_num: usize,
    window: usize,
    last_derivative: f32,
}

impl PidController {
    pub fn new(p: f32, i: f32, d: f32, window: usize) -> Self {
        Self {
            p,
            i,
            d,
            errors: Vec::with_capacity(window),
            time_steps: Vec::with_capacity(window),
            error_num: 0,
            window,
            last_derivative: 0.0,
        }
    }

    pub fn output(&mut self, error: f32, dt: f32) -> f32 {
        if self.errors.len() < self.window {
            self.errors.push(error);
            self.time_steps.push(dt);
        } else {
            self.errors[self.error_num] = error;
            self.time_steps[self.error_num] = dt;
        }
        self.error_num = (self.error_num + 1) % self.window;

        let count = self.errors.len();
        let mut integral = 0.0;
        for i in 0..count {
            integral += self.errors[i] * self.time_steps[i];
        }

        let current = self.errors[(self.error_num + count - 1) % count];
        let previous = self.errors[(self.error_num + count - 2) % count];
        let derivative = (current - previous) / dt.max(f32::EPSILON);

        self.last_derivative = derivative;
        error * self.p + integral * self.i + derivative * self.d
    }

    pub fn reset(&mut self) {
        self.errors.clear();
        self.time_steps.clear();
        self.error_num = 0;
        self.last_derivative = 0.0;
    }
}

impl Default for PidController {
    fn default() -> Self {
        Self::new(1.0, 0.0, 0.0, 10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_tracks_constant_error() {
        let mut pid = PidController::new(1.0, 0.0, 0.0, 5);
        let output = pid.output(5.0, 1.0);
        assert!((output - 5.0).abs() < 0.001);
    }

    #[test]
    fn pid_integral_accumulates() {
        let mut pid = PidController::new(0.0, 1.0, 0.0, 10);
        let a = pid.output(2.0, 1.0);
        let b = pid.output(2.0, 1.0);
        assert!(b > a);
    }

    #[test]
    fn pid_derivative_responds_to_change() {
        let mut pid = PidController::new(0.0, 0.0, 1.0, 5);
        let a = pid.output(0.0, 1.0);
        let b = pid.output(10.0, 1.0);
        assert!((b - a).abs() > 0.001);
    }

    #[test]
    fn pid_reset_clears_history() {
        let mut pid = PidController::new(1.0, 0.0, 0.0, 5);
        pid.output(5.0, 1.0);
        pid.reset();
        let output = pid.output(0.0, 1.0);
        assert!((output - 0.0).abs() < 0.001);
    }

    #[test]
    fn pid_handles_zero_dt() {
        let mut pid = PidController::new(1.0, 0.0, 0.1, 5);
        let output = pid.output(5.0, 0.0);
        assert!((output - 5.0).abs() < 0.001);
    }

    #[test]
    fn pid_derivative_zero_when_single_sample() {
        let mut pid = PidController::new(0.0, 0.0, 1.0, 5);
        let output = pid.output(5.0, 1.0);
        assert!((output - 0.0).abs() < 0.001);
    }
}
