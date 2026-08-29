//! Generates `src/brushsettings_gen.rs` from the vendored libmypaint
//! `brushsettings.json` so the settings/input tables can never drift from
//! upstream. Runtime crates stay dependency-free; this only runs at build
//! time.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let json_path = Path::new("../vendor/libmypaint/brushsettings.json");
    println!("cargo:rerun-if-changed={}", json_path.display());
    let text = fs::read_to_string(json_path).expect("read brushsettings.json");
    let root: serde_json::Value = serde_json::from_str(&text).expect("parse brushsettings.json");

    let mut out = String::new();
    out.push_str("// GENERATED from vendor/libmypaint/brushsettings.json by build.rs — do not edit.\n\n");

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
        let hard_min = opt_f64(&input["hard_minimum"]);
        let hard_max = opt_f64(&input["hard_maximum"]);
        out.push_str(&format!(
            "    InputInfo {{ id: \"{id}\", displayed_name: \"{displayed}\", tooltip: \"{tooltip}\", normal: {normal}, soft_minimum: {smin}, soft_maximum: {smax}, hard_minimum: {hmin}, hard_maximum: {hmax} }},\n",
            id = escape(id),
            displayed = escape(displayed),
            tooltip = escape(tooltip),
            normal = f(x_to_f32_bits(normal)),
            smin = f(x_to_f32_bits(soft_min)),
            smax = f(x_to_f32_bits(soft_max)),
            hmin = opt_bits(hard_min),
            hmax = opt_bits(hard_max),
        ));
    }
    out.push_str("];\n\n");

    out.push_str("/// Setting info table in upstream JSON order (== the C SettingId enum).\n");
    out.push_str("pub static SETTING_INFOS: &[SettingInfo] = &[\n");
    for setting in root["settings"].as_array().expect("settings array") {
        let internal = setting["internal_name"].as_str().unwrap();
        let displayed = setting["displayed_name"].as_str().unwrap();
        let tooltip = setting["tooltip"].as_str().unwrap();
        let constant = setting["constant"].as_bool().unwrap();
        let min = setting["minimum"].as_f64().unwrap();
        let max = setting["maximum"].as_f64().unwrap();
        let def = setting["default"].as_f64().unwrap();
        out.push_str(&format!(
            "    SettingInfo {{ internal_name: \"{i}\", displayed_name: \"{d}\", tooltip: \"{t}\", constant: {c}, minimum: {mn}, maximum: {mx}, default: {df} }},\n",
            i = escape(internal),
            d = escape(displayed),
            t = escape(tooltip),
            c = constant,
            mn = f(x_to_f32_bits(min)),
            mx = f(x_to_f32_bits(max)),
            df = f(x_to_f32_bits(def)),
        ));
    }
    out.push_str("];\n");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest = Path::new(&out_dir).join("brushsettings_gen.rs");
    fs::write(&dest, out).expect("write generated tables");
}

fn x_to_f32_bits(v: f64) -> u32 {
    (v as f32).to_bits()
}

/// Emit an f32 literal built from exact bits: `f32::from_bits(0x...)` keeps
/// the double→float truncation explicit and reviewable.
fn f(bits: u32) -> String {
    format!("f32::from_bits(0x{bits:08x})")
}

fn opt_f64(field: &serde_json::Value) -> Option<f64> {
    field.as_f64()
}

fn opt_bits(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("Some({})", f(x_to_f32_bits(x))),
        None => "None".to_string(),
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
        .replace('\n', "\\n")
}
