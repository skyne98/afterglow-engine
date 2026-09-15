//! Versioned document metadata. Pixel storage and recovery policy stay with the owner.

use super::*;
use serde_json::{Value, json};

fn invalid() -> String { "incorrect paint document metadata".into() }
fn integer(value: &Value) -> Result<i32, String> {
    value.as_i64().and_then(|value| i32::try_from(value).ok()).ok_or_else(invalid)
}
fn boolean(value: &Value) -> Result<bool, String> { value.as_bool().ok_or_else(invalid) }
fn number(value: &Value, minimum: f32, maximum: f32) -> Result<f32, String> {
    let value = value.as_f64().ok_or_else(invalid)?;
    if !value.is_finite() || value < f64::from(minimum) || value > f64::from(maximum) { return Err(invalid()); }
    Ok(value as f32)
}
fn mode(value: &Value) -> Result<BlendMode, String> {
    let mode = integer(value)?;
    if !(0..crate::compositor::BLEND_MODE_COUNT as i32).contains(&mode) { return Err(invalid()); }
    Ok(BlendMode::from_int(mode))
}

impl PaintApp {
    fn document_children(&self, parent: i32) -> Result<Vec<i32>, String> {
        let mut children = Vec::with_capacity(WEB_MAX_LAYERS + WEB_MAX_GROUPS);
        let mut child = self.node_first(parent);
        let mut previous = WEB_REF_NONE;
        while child != WEB_REF_NONE {
            let valid = (child >= 0 && (child as usize) < self.layer_count)
                || ((-(WEB_MAX_GROUPS as i32)..0).contains(&child) && self.group_alive[web_ref_group_id(child)]);
            if !valid || children.len() == WEB_MAX_LAYERS + WEB_MAX_GROUPS
                || self.node_parent(child) != parent || self.node_previous(child) != previous { return Err(invalid()); }
            children.push(child);
            previous = child;
            child = self.node_next(child);
        }
        if previous != self.node_last(parent) { return Err(invalid()); }
        Ok(children)
    }

    /// Save layer order, storage identities, groups, background, and display EOTF.
    /// The result contains no tile pixels, brush settings, or undo records.
    pub fn document_metadata(&self) -> Result<Value, String> {
        let mut layers = Vec::with_capacity(self.layer_count);
        for index in 0..self.layer_count {
            if !self.layer_opacity[index].is_finite() { return Err(invalid()); }
            layers.push(json!({
                "storageId": self.layers[index].storage_id(), "visible": self.layer_visible[index],
                "opacity": self.layer_opacity[index], "mode": self.layer_mode[index] as i32,
            }));
        }
        let mut groups = Vec::with_capacity(WEB_MAX_GROUPS);
        for index in 0..WEB_MAX_GROUPS {
            if !self.group_alive[index] { groups.push(Value::Null); continue; }
            if !self.group_opacity[index].is_finite() { return Err(invalid()); }
            groups.push(json!({
                "visible": self.group_visible[index], "opacity": self.group_opacity[index],
                "mode": self.group_mode[index] as i32, "passThrough": self.group_pass_through[index],
                "isolated": self.group_isolated[index], "children": self.document_children(index as i32)?,
            }));
        }
        Ok(json!({
            "version": 1, "width": self.width, "height": self.height, "activeLayer": self.active_layer,
            "background": self.background_color, "eotf": self.display_eotf,
            "layers": layers, "groups": groups, "roots": self.document_children(-1)?,
        }))
    }

    /// Build a separate empty document. Invalid metadata never changes a live app.
    /// The owner must attach storage and register saved tile keys before publication.
    pub fn from_document_metadata(metadata: &Value, initial: usize, maximum: usize) -> Result<Self, String> {
        if metadata["version"].as_u64() != Some(1) { return Err(invalid()); }
        let width = integer(&metadata["width"])?;
        let height = integer(&metadata["height"])?;
        if !(1..=16384).contains(&width) || !(1..=16384).contains(&height) { return Err(invalid()); }
        let layers = metadata["layers"].as_array().ok_or_else(invalid)?;
        let groups = metadata["groups"].as_array().ok_or_else(invalid)?;
        let active = integer(&metadata["activeLayer"])?;
        if layers.is_empty() || layers.len() > WEB_MAX_LAYERS || groups.len() != WEB_MAX_GROUPS
            || active < 0 || active as usize >= layers.len() { return Err(invalid()); }
        let background = metadata["background"].as_array().filter(|array| array.len() == 4).ok_or_else(invalid)?;
        let mut color = [0u16; 4];
        for (channel, value) in color.iter_mut().zip(background) {
            let value = value.as_u64().filter(|&value| value <= 32768).ok_or_else(invalid)?;
            *channel = value as u16;
        }
        if color[..3].iter().any(|&channel| channel > color[3]) { return Err(invalid()); }
        let eotf = number(&metadata["eotf"], 0.0, f32::MAX)?;
        if eotf == 0.0 { return Err(invalid()); }
        let mut app = Self::new_with_tile_limits(width, height, initial, maximum).ok_or("paint document allocation failed")?;
        while app.layer_count < layers.len() {
            if app.create_layer() < 0 { return Err("paint layer allocation failed".into()); }
        }
        let mut ids = [0u32; WEB_MAX_LAYERS];
        for (index, layer) in layers.iter().enumerate() {
            let id = layer["storageId"].as_u64().and_then(|id| u32::try_from(id).ok()).ok_or_else(invalid)?;
            if ids[..index].contains(&id) || !app.layers[index].restore_storage_id(id) { return Err(invalid()); }
            ids[index] = id;
            app.layer_visible[index] = boolean(&layer["visible"])?;
            app.layer_opacity[index] = number(&layer["opacity"], 0.0, 1.0)?;
            app.layer_mode[index] = mode(&layer["mode"])?;
        }
        for (index, group) in groups.iter().enumerate() {
            if group.is_null() { continue; }
            app.group_alive[index] = true;
            app.group_count = index + 1;
            app.group_visible[index] = boolean(&group["visible"])?;
            app.group_pass_through[index] = boolean(&group["passThrough"])?;
            app.group_isolated[index] = boolean(&group["isolated"])?;
            app.group_opacity[index] = number(&group["opacity"], 0.0, 1.0)?;
            app.group_mode[index] = mode(&group["mode"])?;
        }
        app.root_first_child = WEB_REF_NONE;
        app.root_last_child = WEB_REF_NONE;
        app.layer_parent.fill(-2);
        app.layer_next.fill(WEB_REF_NONE);
        app.layer_previous.fill(WEB_REF_NONE);
        let mut seen_layers = [false; WEB_MAX_LAYERS];
        let mut seen_groups = [false; WEB_MAX_GROUPS];
        app.restore_children(&metadata["roots"], -1, groups, &mut seen_layers, &mut seen_groups)?;
        if seen_layers[..layers.len()].iter().any(|&seen| !seen) || seen_groups != app.group_alive { return Err(invalid()); }
        app.active_layer = active as usize;
        app.background_color = color;
        for pixel in app.background_tile.chunks_exact_mut(4) { pixel.copy_from_slice(&color); }
        app.set_eotf(eotf);
        Ok(app)
    }

    fn restore_children(&mut self, children: &Value, parent: i32, groups: &[Value],
        seen_layers: &mut [bool; WEB_MAX_LAYERS], seen_groups: &mut [bool; WEB_MAX_GROUPS]) -> Result<(), String> {
        let children = children.as_array().filter(|children| children.len() <= WEB_MAX_LAYERS + WEB_MAX_GROUPS).ok_or_else(invalid)?;
        for child in children {
            let child = integer(child)?;
            if child >= 0 && (child as usize) < self.layer_count {
                if std::mem::replace(&mut seen_layers[child as usize], true) { return Err(invalid()); }
                self.node_append(child, parent);
            } else if (-(WEB_MAX_GROUPS as i32)..0).contains(&child) {
                let group = web_ref_group_id(child);
                if !self.group_alive[group] || std::mem::replace(&mut seen_groups[group], true) { return Err(invalid()); }
                self.node_append(child, parent);
                self.restore_children(&groups[group]["children"], group as i32, groups, seen_layers, seen_groups)?;
            } else { return Err(invalid()); }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_restores_tree_properties_and_storage_ids_without_pixels() {
        let mut app = PaintApp::new(128, 256).unwrap();
        app.create_layer();
        app.create_layer();
        assert!(app.delete_layer(1));
        let outer = app.create_group();
        let inner = app.create_group();
        assert!(app.set_group_parent(inner as usize, outer));
        assert!(app.set_layer_group(0, outer));
        assert!(app.set_layer_group(1, inner));
        app.group_pass_through[0] = true;
        app.group_isolated[0] = false;
        app.group_opacity[1] = 0.7;
        app.set_layer_mode(1, BlendMode::Normal);
        app.set_layer_opacity(0, 0.6);
        app.set_layer_visible(1, false);
        app.set_active_layer(0);
        app.set_eotf(1.8);
        app.clear_background();
        app.active().get_or_create_tile_mut(0, 0).unwrap().fill(100);
        let metadata = app.document_metadata().unwrap();
        let mut restored = PaintApp::from_document_metadata(&metadata, 1, 2).unwrap();
        assert_eq!(restored.document_metadata().unwrap(), metadata);
        assert_eq!(restored.resident_tile_count(), 0);
        let previous = restored.layers.iter().map(WebSurface::storage_id).max().unwrap();
        let next = restored.create_layer();
        assert!(restored.layers[next as usize].storage_id() > previous);
    }

    #[test]
    fn metadata_rejects_invalid_dimensions_identities_modes_and_tree_links() {
        let mut app = PaintApp::new(64, 64).unwrap();
        app.create_layer();
        app.create_group();
        let original = app.document_metadata().unwrap();
        let invalid_values = [
            ("/version", json!(2)), ("/width", json!(16385)), ("/height", json!(-1)),
            ("/activeLayer", json!(2)), ("/layers/1/storageId", json!(1)),
            ("/layers/0/storageId", json!(4294967295u64)), ("/layers/0/opacity", json!(1.1)),
            ("/layers/0/mode", json!(99)), ("/layers/0/visible", json!("true")),
            ("/background/3", json!(1)), ("/eotf", json!(0)),
            ("/roots", json!([0, 0, -1])), ("/roots", json!([0, -1])),
            ("/roots", json!([0, 1, -2])), ("/roots", json!([0, 1, -2147483648i64])),
            ("/groups/0/children", json!([-1])), ("/groups/0/children", json!([1])),
        ];
        for (path, value) in invalid_values {
            let mut metadata = original.clone();
            *metadata.pointer_mut(path).unwrap() = value;
            assert!(PaintApp::from_document_metadata(&metadata, 1, 2).is_err(), "{path}");
            assert_eq!(app.document_metadata().unwrap(), original);
        }
    }

    #[test]
    fn nested_groups_and_pass_through_use_one_tile_of_scratch_per_level() {
        let mut app = PaintApp::new(64, 64).unwrap();
        app.set_layer_mode(0, BlendMode::Normal);
        app.layers[0].get_or_create_tile_mut(0, 0).unwrap().fill(12000);
        let expected = app.render_tile(0, 0).to_vec();
        for group in 0..WEB_MAX_GROUPS {
            assert_eq!(app.create_group(), group as i32);
            if group > 0 { assert!(app.set_group_parent(group, group as i32 - 1)); }
        }
        assert!(app.set_layer_group(0, WEB_MAX_GROUPS as i32 - 1));
        assert_eq!(app.render_tile(0, 0), expected);
        for group in 0..WEB_MAX_GROUPS {
            app.group_pass_through[group] = true;
            app.group_isolated[group] = false;
        }
        assert_eq!(app.render_tile(0, 0), expected);
    }

    #[test]
    fn layer_deletion_keeps_group_links_order_and_pixels() {
        let mut app = PaintApp::new(64, 64).unwrap();
        app.create_layer();
        app.create_layer();
        let group = app.create_group();
        assert!(app.set_layer_group(2, group));
        assert!(app.set_layer_group(0, group));
        app.set_layer_mode(2, BlendMode::Normal);
        app.layers[2].get_or_create_tile_mut(0, 0).unwrap().fill(16000);
        let expected = app.render_tile(0, 0).to_vec();
        assert!(app.delete_layer(1));
        assert_eq!(app.document_children(group).unwrap(), vec![1, 0]);
        assert_eq!(app.render_tile(0, 0), expected);
        assert!(!app.set_layer_group(0, -2));
        assert!(!app.set_group_parent(group as usize, -2));
        assert!(app.move_layer(0, -1));
        assert_eq!(app.document_children(group).unwrap(), vec![0, 1]);
        assert!(app.document_metadata().is_ok());
    }
}
