//! Shared .myb v3 JSON loader (native + wasm).
use crate::brush::Brush;

/// `.myb` v3 JSON loader — mirrors `update_brush_from_json_object`:
/// version must be 3, each known setting applies base_value + inputs, and
/// the call succeeds when at least one setting applied.
pub fn load(brush: &mut Brush, text: &str) -> bool {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    if root.get("version").and_then(|v| v.as_i64()) != Some(3) {
        return false;
    }
    let Some(settings) = root.get("settings") else {
        return false;
    };
    let Some(settings) = settings.as_object() else {
        return false;
    };
    let mut updated_any = false;
    for (setting_name, setting_obj) in settings {
        let Some(setting_index) = crate::settings::setting_index_runtime(setting_name) else {
            continue; // unknown setting: warn-free skip, keep scanning
        };
        let Some(base_value) = setting_obj.get("base_value").and_then(|v| v.as_f64()) else {
            continue;
        };
        brush.set_base_value_at(setting_index, base_value as f32);
        updated_any = true;
        let Some(inputs) = setting_obj.get("inputs").and_then(|v| v.as_object()) else {
            continue;
        };
        for (input_name, input_obj) in inputs {
            let Some(input_index) = crate::settings::input_index_runtime(input_name) else {
                continue;
            };
            let Some(points) = input_obj.as_array() else {
                continue;
            };
            brush.set_mapping_n_at(setting_index, input_index, points.len());
            for (i, point) in points.iter().enumerate() {
                let Some(point) = point.as_array() else { continue };
                if point.len() < 2 {
                    continue;
                }
                let x = point[0].as_f64().unwrap_or(0.0) as f32;
                let y = point[1].as_f64().unwrap_or(0.0) as f32;
                brush.set_mapping_point_at(setting_index, input_index, i, x, y);
            }
        }
    }
    updated_any
}

