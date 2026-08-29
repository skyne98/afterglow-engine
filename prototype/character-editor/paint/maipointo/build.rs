//! Generates `src/brushsettings_gen.rs` from the vendored libmypaint
//! `brushsettings.json` (and the states enum from the committed C gen
//! header) so the tables can never drift from upstream. Runtime crates stay
//! dependency-free; this only runs at build time.

use std::env;
use std::fs;
use std::path::Path;

/// `internal_name` → Rust variant (snake_case → UpperCamelCase).
fn pascal(name: &str) -> String {
    let mut out = String::new();
    for part in name.split('_') {
        if part.is_empty() {
            continue;
        }
        // Keep leading acronyms sane: h, s, v → H, S, V; x1 → X1; dtime → Dtime
        let mut chars = part.chars();
        let first = chars.next().unwrap().to_ascii_uppercase();
        out.push(first as char);
        out.push_str(chars.as_str());
    }
    out
}

fn main() {
    let json_path = Path::new("../vendor/libmypaint/brushsettings.json");
    println!("cargo:rerun-if-changed={}", json_path.display());
    let text = fs::read_to_string(json_path).expect("read brushsettings.json");
    let root: serde_json::Value = serde_json::from_str(&text).expect("parse brushsettings.json");

    // States come from the committed C gen header (not in the JSON).
    let gen_header =
        fs::read_to_string("../vendor/libmypaint/mypaint-brush-settings-gen.h")
            .expect("read mypaint-brush-settings-gen.h");
    let mut states: Vec<String> = Vec::new();
    for line in gen_header.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("MYPAINT_BRUSH_STATE_") {
            if let Some(name) = rest.strip_suffix(',') {
                if name != "COUNT" {
                    states.push(name.to_string());
                }
            }
        }
    }

    let mut out = String::new();
    out.push_str(
        "// GENERATED from vendor/libmypaint/brushsettings.json by build.rs — do not edit.\n\n",
    );

    // Inputs (JSON order == MyPaintInputId order in the C enum).
    out.push_str("/// Input info table in upstream `brushsettings.json` order.\n");
    out.push_str("pub static INPUT_INFOS: &[InputInfo] = &[\n");
    for input in root["inputs"].as_array().expect("inputs array") {
        let id = input["id"].as_str().unwrap();
        let displayed = input["displayed_name"].as_str().unwrap();
        let tooltip = input["tooltip"].as_str().unwrap();
        let normal = input["normal"].as_f64().unwrap();
        let soft_min = input["soft_minimum"].as_f64().unwrap();
        let soft_max = input["soft_maximum"].as_f64().unwrap();
        let hard_min = input["hard_minimum"].as_f64();
        let hard_max = input["hard_maximum"].as_f64();
        out.push_str(&format!(
            "    InputInfo {{ id: \"{i}\", displayed_name: \"{d}\", tooltip: \"{t}\", normal: {n}, soft_minimum: {smin}, soft_maximum: {smax}, hard_minimum: {hmin}, hard_maximum: {hmax} }},\n",
            i = escape(id),
            d = escape(input["displayed_name"].as_str().unwrap()),
            t = escape(input["tooltip"].as_str().unwrap()),
            n = f(normal),
            smin = f(soft_min),
            smax = f(soft_max),
            hmin = opt(hard_min),
            hmax = opt(hard_max),
        ));
    }
    out.push_str("];\n\n");

    out.push_str("/// Setting info table in upstream JSON order (== the C SettingId enum).\n");
    out.push_str("pub static SETTING_INFOS: &[SettingInfo] = &[\n");
    for setting in root["settings"].as_array().expect("settings array") {
        out.push_str(&format!(
            "    SettingInfo {{ internal_name: \"{i}\", displayed_name: \"{d}\", tooltip: \"{t}\", constant: {c}, minimum: {mn}, maximum: {mx}, default: {df} }},\n",
            i = escape(setting["internal_name"].as_str().unwrap()),
            d = escape(setting["displayed_name"].as_str().unwrap()),
            t = escape(setting["tooltip"].as_str().unwrap()),
            c = setting["constant"].as_bool().unwrap(),
            mn = f(setting["minimum"].as_f64().unwrap()),
            mx = f(setting["maximum"].as_f64().unwrap()),
            df = f(setting["default"].as_f64().unwrap()),
        ));
    }
    out.push_str("];\n\n");

    // SettingId / InputId / BrushStateId enums (orders match the C enums).
    out.push_str("/// Upstream `MyPaintBrushSetting` order (JSON order).\n#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n#[allow(missing_docs)]\npub enum SettingId {\n");
    for setting in root["settings"].as_array().unwrap() {
        out.push_str(&format!(
            "    {},\n",
            pascal(setting["internal_name"].as_str().unwrap())
        ));
    }
    out.push_str("}\n\nimpl SettingId {\n    pub const fn index(self) -> usize {\n        self as usize\n    }\n}\n\n");

    out.push_str("/// Upstream `MyPaintBrushInput` order (JSON order).\n#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n#[allow(missing_docs)]\npub enum InputId {\n");
    for input in root["inputs"].as_array().unwrap() {
        out.push_str(&format!("    {},\n", pascal(input["id"].as_str().unwrap())));
    }
    out.push_str("}\n\nimpl InputId {\n    pub const fn index(self) -> usize {\n        self as usize\n    }\n}\n\n");

    out.push_str("/// Upstream `MyPaintBrushState` order (from mypaint-brush-settings-gen.h).\n#[derive(Clone, Copy, Debug, PartialEq, Eq)]\n#[allow(missing_docs)]\npub enum BrushStateId {\n");
    for state in &states {
        out.push_str(&format!("    {},\n", pascal(&state.to_ascii_lowercase())));
    }
    out.push_str("}\n\nimpl BrushStateId {\n    pub const fn index(self) -> usize {\n        self as usize\n    }\n}\n\n/// Number of brush states (`MYPAINT_BRUSH_STATES_COUNT`).\npub const BRUSH_STATES_COUNT: usize = ");
    out.push_str(&states.len().to_string());
    out.push_str(";\n");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("brushsettings_gen.rs");
    fs::write(&dest, out).expect("write generated tables");
}

/// Exact double→f32 truncation, emitted as explicit bits.
fn f(v: f64) -> String {
    format!("f32::from_bits(0x{:08x})", (v as f32).to_bits())
}

fn opt(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("Some({})", f(x)),
        None => "None".to_string(),
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}
