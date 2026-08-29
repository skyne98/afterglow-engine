//! Settings/input info tables — generated at build time from the vendored
//! `brushsettings.json` (see `build.rs`).

include!(concat!(env!("OUT_DIR"), "/brushsettings_gen.rs"));

/// Number of brush inputs (the `MyPaintInputId` count).
pub const INPUTS: usize = INPUT_INFOS.len();

/// Number of brush settings (the `MyPaintBrushSettingId` count).
pub const SETTINGS: usize = SETTING_INFOS.len();

/// Input info (upstream `brushsettings.json` order).
#[derive(Debug)]
pub struct InputInfo {
    pub id: &'static str,
    pub displayed_name: &'static str,
    pub tooltip: &'static str,
    pub normal: f32,
    pub soft_minimum: f32,
    pub soft_maximum: f32,
    pub hard_minimum: Option<f32>,
    pub hard_maximum: Option<f32>,
}

/// Setting info (JSON order == the C `MyPaintBrushSettingId` enum order).
#[derive(Debug)]
pub struct SettingInfo {
    pub internal_name: &'static str,
    pub displayed_name: &'static str,
    pub tooltip: &'static str,
    pub constant: bool,
    pub minimum: f32,
    pub maximum: f32,
    pub default: f32,
}

/// Index of an input by upstream `id` (compile-time-checked unique names).
pub const fn input_index(id: &str) -> Option<usize> {
    let mut i = 0;
    while i < INPUT_INFOS.len() {
        if str_eq(INPUT_INFOS[i].id, id) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Index of a setting by upstream `internal_name`.
pub const fn setting_index(internal_name: &str) -> Option<usize> {
    let mut i = 0;
    while i < SETTING_INFOS.len() {
        if str_eq(SETTING_INFOS[i].internal_name, internal_name) {
            return Some(i);
        }
        i += 1;
    }
    None
}

const fn str_eq(a: &str, b: &str) -> bool {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    if ab.len() != bb.len() {
        return false;
    }
    let mut i = 0;
    while i < ab.len() {
        if ab[i] != bb[i] {
            return false;
        }
        i += 1;
    }
    true
}
