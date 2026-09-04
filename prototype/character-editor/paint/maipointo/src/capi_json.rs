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
            // The C `mypaint_mapping_set_n`/`set_point` guards (g_return_if_fail)
            // skip a 1-point curve and a non-monotonic point but keep loading
            // the rest of the brush; the Rust Mapping asserts instead, so
            // filter here to keep a malformed `.myb` from aborting the load.
            if points.len() < 2 {
                continue;
            }
            brush.set_mapping_n_at(setting_index, input_index, points.len());
            let mut prev_x = f32::NEG_INFINITY;
            for (i, point) in points.iter().enumerate() {
                let Some(point) = point.as_array() else {
                    continue;
                };
                if point.len() < 2 {
                    continue;
                }
                let x = point[0].as_f64().unwrap_or(0.0) as f32;
                let y = point[1].as_f64().unwrap_or(0.0) as f32;
                if x < prev_x {
                    continue;
                }
                prev_x = x;
                brush.set_mapping_point_at(setting_index, input_index, i, x, y);
            }
        }
    }
    updated_any
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Brush {
        let mut brush = Brush::new();
        brush.from_defaults();
        brush
    }

    #[test]
    fn malformed_curves_are_skipped_not_fatal() {
        // 1-point curve (set_n would assert) + non-monotonic points
        // (set_point would assert): the load must succeed and keep the rest.
        let json = r#"{
            "version": 3,
            "settings": {
                "radius_logarithmic": {"base_value": 2.5},
                "opaque_multiply": {
                    "base_value": 1.0,
                    "inputs": {
                        "pressure": [[0.0, 0.0]],
                        "speed1": [[1.0, 0.5], [0.25, 1.0], [0.5, 1.0], [1.0, 0.9]]
                    }
                }
            }
        }"#;
        let mut brush = defaults();
        assert!(load(&mut brush, json));
        let radius = crate::settings::setting_index_runtime("radius_logarithmic").unwrap();
        let radius_id = crate::settings::SettingId::from_index(radius).unwrap();
        assert_eq!(brush.get_base_value(radius_id), 2.5);
    }

    #[test]
    fn wrong_version_is_rejected() {
        let mut brush = defaults();
        assert!(!load(&mut brush, "{\"version\": 2, \"settings\": {}}"));
    }
}
