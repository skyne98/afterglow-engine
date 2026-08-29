//! Symmetry — port of `mypaint-symmetry.c` + `mypaint-matrix.c` (3x3 f32
//! row-vector transforms) with the demo's `draw_dab` symmetry passes and
//! `prepare_bounding_boxes`. Transcribed bit-for-bit, including the
//!Snowflake fall-through into the Rotational pass.

pub const MYPAINT_TILE_SIZE: usize = 64;
/// `NUM_BBOXES_DEFAULT` from mypaint-tiled-surface.h.
pub const NUM_BBOXES_DEFAULT: usize = 8;
/// Fixed matrix store: covers any line count the demo uses; the C grows a
/// realloc'd array instead. 2 x 64 lines is a hard, deterministic bound.
pub const MAX_SYMMETRY_MATRICES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum SymmetryType {
    Vertical = 0,
    Horizontal = 1,
    VertHorz = 2,
    Rotational = 3,
    Snowflake = 4,
}

/// 3x3 row-major transform (`mypaint-matrix.c`).
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    pub rows: [[f32; 3]; 3],
}

impl Transform {
    /// All-zero transform (the C calloc's its matrix store).
    pub const ZERO: Transform = Transform {
        rows: [[0.0; 3]; 3],
    };
}

impl Default for Transform {
    fn default() -> Self {
        Transform::unit()
    }
}

impl Transform {
    pub fn unit() -> Self {
        Transform {
            rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    /// `mypaint_matrix_multiply` (note the C's index order).
    #[must_use]
    pub fn multiply(m1: Transform, m2: Transform) -> Transform {
        let mut result = Transform::unit();
        for row in 0..3 {
            for col in 0..3 {
                result.rows[row][col] = m1.rows[0][col] * m2.rows[row][0]
                    + m1.rows[1][col] * m2.rows[row][1]
                    + m1.rows[2][col] * m2.rows[row][2];
            }
        }
        result
    }

    /// `mypaint_transform_rotate_cw`.
    #[must_use]
    pub fn rotate_cw(transform: Transform, angle_radians: f32) -> Transform {
        let a = angle_radians;
        let factor = Transform {
            rows: [
                [a.cos(), a.sin(), 0.0],
                [-a.sin(), a.cos(), 0.0],
                [0.0, 0.0, 1.0],
            ],
        };
        Transform::multiply(transform, factor)
    }

    /// `mypaint_transform_reflect`.
    #[must_use]
    pub fn reflect(transform: Transform, angle_radians: f32) -> Transform {
        let x = angle_radians.cos();
        let y = angle_radians.sin();
        let factor = Transform {
            rows: [
                [x * x - y * y, 2.0 * x * y, 0.0],
                [2.0 * x * y, y * y - x * x, 0.0],
                [0.0, 0.0, 1.0],
            ],
        };
        Transform::multiply(transform, factor)
    }

    /// `mypaint_transform_translate`.
    #[must_use]
    pub fn translate(transform: Transform, x: f32, y: f32) -> Transform {
        let factor = Transform {
            rows: [[1.0, 0.0, x], [0.0, 1.0, y], [0.0, 0.0, 1.0]],
        };
        Transform::multiply(transform, factor)
    }

    /// `mypaint_transform_point`.
    pub fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.rows[0][0] * x + self.rows[0][1] * y + self.rows[0][2],
            self.rows[1][0] * x + self.rows[1][1] * y + self.rows[1][2],
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SymmetryState {
    pub kind: i32, // SymmetryType as the C int; -1 = unset
    pub center_x: f32,
    pub center_y: f32,
    pub angle: f32,
    pub num_lines: i32,
}

impl Default for SymmetryState {
    fn default() -> Self {
        SymmetryState {
            kind: 0, // MYPAINT_SYMMETRY_TYPE_VERTICAL
            center_x: 0.0,
            center_y: 0.0,
            angle: 0.0,
            num_lines: 2,
        }
    }
}

/// `mypaint-symmetry.c` state machinery (matrices in a fixed store).
pub struct SymmetryData {
    pub state_current: SymmetryState,
    pub state_pending: SymmetryState,
    pub pending_changes: bool,
    pub active: bool,
    pub matrices: [Transform; MAX_SYMMETRY_MATRICES],
    pub num_symmetry_matrices: usize,
}

/// `num_matrices_required`.
fn num_matrices_required(state: &SymmetryState) -> usize {
    match state.kind {
        0 | 1 => 1,                       // vertical / horizontal
        2 => 3,                           // verthorz
        3 => (state.num_lines - 1) as usize, // rotational
        4 => (2 * state.num_lines - 1) as usize, // snowflake
        _ => 0,
    }
}

/// `mypaint_update_symmetry_state` — the C transcribes through f32 `cos`/
/// `sin` (double libm narrowed to float parameters) and float matrix math.
pub fn update_symmetry_state(data: &mut SymmetryData) {
    if !data.pending_changes || data.state_current == data.state_pending {
        return;
    }
    let required = num_matrices_required(&data.state_pending);
    if required > MAX_SYMMETRY_MATRICES {
        return; // deterministic bound (the C would realloc here)
    }
    let symm = data.state_pending;
    data.state_current = symm;
    let cx = symm.center_x;
    let cy = symm.center_y;
    // C: `float angle = symm.angle * (M_PI / 180.0);` — the double product
    // narrows to the float variable; `angle += M_PI / 2.0` re-widens then
    // re-narrows. `float rot_angle = (2.0 * M_PI) / num_lines` likewise.
    let mut angle = ((symm.angle as f64) * (std::f64::consts::PI / 180.0)) as f32;
    let rot_angle = ((2.0f64 * std::f64::consts::PI) / symm.num_lines as f64) as f32;
    let m = Transform::translate(Transform::unit(), -cx, -cy);
    match symm.kind {
        0 | 1 => {
            if symm.kind == 0 {
                angle = (angle as f64 + std::f64::consts::PI / 2.0) as f32;
            }
            data.matrices[0] = Transform::reflect(m, -angle);
        }
        2 => {
            let v_angle = (angle as f64 + std::f64::consts::PI / 2.0) as f32;
            data.matrices[0] = Transform::reflect(m, -angle);
            data.matrices[1] = Transform::reflect(data.matrices[0], -v_angle);
            data.matrices[2] = Transform::reflect(data.matrices[1], -angle);
        }
        4 => {
            // Snowflake: no break in the C — falls through to rotational.
            let base_idx = (symm.num_lines - 1) as usize;
            for i in 0..symm.num_lines as usize {
                data.matrices[base_idx + i] = Transform::reflect(
                    Transform::rotate_cw(m, rot_angle * i as f32),
                    -(i as f32) * rot_angle - angle,
                );
            }
            for i in 1..symm.num_lines as usize {
                data.matrices[i - 1] = Transform::rotate_cw(m, rot_angle * i as f32);
            }
        }
        3 => {
            for i in 1..symm.num_lines as usize {
                data.matrices[i - 1] = Transform::rotate_cw(m, rot_angle * i as f32);
            }
        }
        _ => {}
    }
    for i in 0..required {
        data.matrices[i] = Transform::translate(data.matrices[i], cx, cy);
    }
    // The C keeps the allocated size (16 initially, grown on demand).
    data.num_symmetry_matrices = data.num_symmetry_matrices.max(required);
    data.pending_changes = false;
}

/// `mypaint_default_symmetry_data` (initial store 16 like the C).
pub fn default_symmetry_data() -> SymmetryData {
    let mut data = SymmetryData {
        state_current: SymmetryState {
            kind: -1,
            center_x: 0.0,
            center_y: 0.0,
            angle: 0.0,
            num_lines: 2,
        },
        state_pending: SymmetryState::default(),
        pending_changes: true,
        active: false,
        // The C callocs its matrix store — zeroed, not unit.
        matrices: [Transform::ZERO; MAX_SYMMETRY_MATRICES],
        num_symmetry_matrices: 16,
    };
    update_symmetry_state(&mut data);
    data
}

/// `mypaint_symmetry_set_pending`.
pub fn symmetry_set_pending(
    data: &mut SymmetryData,
    active: bool,
    center_x: f32,
    center_y: f32,
    symmetry_angle: f32,
    symmetry_type: i32,
    rot_symmetry_lines: i32,
) {
    data.active = active;
    data.state_pending.center_x = center_x;
    data.state_pending.center_y = center_y;
    data.state_pending.kind = symmetry_type;
    data.state_pending.num_lines = 2.max(rot_symmetry_lines);
    data.state_pending.angle = symmetry_angle;
    data.pending_changes = true;
}

/// One dirty rectangle (`MyPaintRectangle`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rectangle {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// `mypaint_rectangle_expand_to_include_point`.
pub fn rectangle_expand_to_include_point(r: &mut Rectangle, x: i32, y: i32) {
    if r.width == 0 {
        r.width = 1;
        r.height = 1;
        r.x = x;
        r.y = y;
    } else {
        if x < r.x {
            r.width += r.x - x;
            r.x = x;
        } else if x >= r.x + r.width {
            r.width = x - r.x + 1;
        }
        if y < r.y {
            r.height += r.y - y;
            r.y = y;
        } else if y >= r.y + r.height {
            r.height = y - r.y + 1;
        }
    }
}

/// `mypaint_rectangle_expand_to_include_rect`.
pub fn rectangle_expand_to_include_rect(r: &mut Rectangle, other: &Rectangle) {
    rectangle_expand_to_include_point(r, other.x, other.y);
    rectangle_expand_to_include_point(r, other.x + other.width - 1, other.y + other.height - 1);
}
