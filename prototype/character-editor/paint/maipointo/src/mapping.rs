//! User-defined input→setting mappings — port of `mypaint-mapping.c`.
//!
//! The curves editable in the brush settings: a base value plus one optional
//! stepwise-linear control-point curve per input. All arithmetic is `f32`,
//! matching the C exactly.

/// Maximum control points per input (`ControlPoints` in the C).
pub const MAPPING_MAX_POINTS: usize = 64;

#[derive(Clone, Copy, Debug)]
struct ControlPoints {
    /// A set of control points (stepwise linear).
    xvalues: [f32; MAPPING_MAX_POINTS],
    yvalues: [f32; MAPPING_MAX_POINTS],
    n: usize,
}

impl Default for ControlPoints {
    fn default() -> Self {
        Self {
            xvalues: [0.0; MAPPING_MAX_POINTS],
            yvalues: [0.0; MAPPING_MAX_POINTS],
            n: 0,
        }
    }
}

/// `MyPaintMapping` — one optional curve per input, added on top of a base.
#[derive(Debug)]
pub struct Mapping {
    base_value: f32,
    points_list: Vec<ControlPoints>,
    inputs_used: usize,
}

impl Mapping {
    /// `mypaint_mapping_new`.
    pub fn new(inputs: usize) -> Self {
        Self {
            base_value: 0.0,
            points_list: vec![ControlPoints::default(); inputs],
            inputs_used: 0,
        }
    }

    pub fn inputs(&self) -> usize {
        self.points_list.len()
    }

    pub fn get_base_value(&self) -> f32 {
        self.base_value
    }

    pub fn set_base_value(&mut self, value: f32) {
        self.base_value = value;
    }

    /// `mypaint_mapping_set_n`. Panics mirror the C asserts:
    /// `input` in range, `n <= 64`, and `n != 1` (one point cannot build a
    /// linear mapping).
    pub fn set_n(&mut self, input: usize, n: usize) {
        assert!(input < self.points_list.len());
        assert!(n <= MAPPING_MAX_POINTS);
        assert!(n != 1);
        let p = &mut self.points_list[input];
        if n != 0 && p.n == 0 {
            self.inputs_used += 1;
        }
        if n == 0 && p.n != 0 {
            self.inputs_used -= 1;
        }
        p.n = n;
    }

    /// `mypaint_mapping_get_n`.
    pub fn get_n(&self, input: usize) -> usize {
        self.points_list[input].n
    }

    /// `mypaint_mapping_set_point`.
    pub fn set_point(&mut self, input: usize, index: usize, x: f32, y: f32) {
        assert!(input < self.points_list.len());
        assert!(index < MAPPING_MAX_POINTS);
        let p = &mut self.points_list[input];
        assert!(index < p.n);
        if index > 0 {
            assert!(x >= p.xvalues[index - 1]);
        }
        p.xvalues[index] = x;
        p.yvalues[index] = y;
    }

    /// `mypaint_mapping_get_point`.
    pub fn get_point(&self, input: usize, index: usize) -> (f32, f32) {
        assert!(input < self.points_list.len());
        assert!(index < MAPPING_MAX_POINTS);
        let p = &self.points_list[input];
        assert!(index < p.n);
        (p.xvalues[index], p.yvalues[index])
    }

    /// `mypaint_mapping_is_constant`.
    pub fn is_constant(&self) -> bool {
        self.inputs_used == 0
    }

    /// `mypaint_mapping_get_inputs_used_n`.
    pub fn inputs_used(&self) -> usize {
        self.inputs_used
    }

    /// `mypaint_mapping_calculate` — base value plus every curve evaluated
    /// at `data[input]`. Segment search and interpolation match the C.
    pub fn calculate(&self, data: &[f32]) -> f32 {
        let mut result = self.base_value;

        // Constant mapping (common case).
        if self.inputs_used == 0 {
            return result;
        }

        for (j, p) in self.points_list.iter().enumerate() {
            if p.n != 0 {
                let x = data[j];

                // find the segment with the slope that we need to use
                let mut x0 = p.xvalues[0];
                let mut y0 = p.yvalues[0];
                let mut x1 = p.xvalues[1];
                let mut y1 = p.yvalues[1];

                let mut i = 2;
                while i < p.n && x > x1 {
                    x0 = x1;
                    y0 = y1;
                    x1 = p.xvalues[i];
                    y1 = p.yvalues[i];
                    i += 1;
                }

                let y;
                if x0 == x1 || y0 == y1 {
                    y = y0;
                } else {
                    // linear interpolation
                    y = (y1 * (x - x0) + y0 * (x1 - x)) / (x1 - x0);
                }

                result += y;
            }
        }
        result
    }

    /// `mypaint_mapping_calculate_single_input`.
    pub fn calculate_single_input(&self, input: f32) -> f32 {
        assert_eq!(self.points_list.len(), 1);
        self.calculate(&[input])
    }
}
