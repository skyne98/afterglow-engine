use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const GLB_MAGIC: u32 = 0x46546c67;
const JSON_CHUNK: u32 = 0x4e4f534a;
const BIN_CHUNK: u32 = 0x004e4942;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlbImage {
    pub index: usize,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlbJson {
    #[serde(default)]
    buffer_views: Vec<BufferView>,
    #[serde(default)]
    images: Vec<Image>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BufferView {
    buffer: usize,
    #[serde(default)]
    byte_offset: usize,
    byte_length: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Image {
    buffer_view: Option<usize>,
    mime_type: Option<String>,
    uri: Option<String>,
}

fn confined_dependency(root: &Path, uri: &str) -> Result<PathBuf, String> {
    if uri.starts_with("data:") {
        return Err("data URIs are not supported by the GLTF cook".into());
    }
    let candidate = root.join(uri);
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("canonicalize GLTF root: {error}"))?;
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("read GLTF dependency {uri}: {error}"))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(format!("GLTF dependency escapes source root: {uri}"));
    }
    Ok(canonical)
}

fn mime_for_uri(uri: &str) -> Result<&'static str, String> {
    let lowercase = uri.to_ascii_lowercase();
    if lowercase.ends_with(".png") {
        Ok("image/png")
    } else if lowercase.ends_with(".jpg") || lowercase.ends_with(".jpeg") {
        Ok("image/jpeg")
    } else {
        Err(format!("unsupported GLTF image type: {uri}"))
    }
}

/// Convert an external `.gltf` plus side files into one self-contained GLB.
/// This is the normal cook path for downloaded Khronos packages.
pub fn embed_external_gltf(path: &Path) -> Result<Vec<u8>, String> {
    let root = path.parent().ok_or("GLTF path has no parent")?;
    let mut document: Value = serde_json::from_slice(
        &std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", path.display()))?;
    let object = document
        .as_object_mut()
        .ok_or("GLTF document must be an object")?;
    let buffer_uri = object
        .get("buffers")
        .and_then(Value::as_array)
        .filter(|buffers| buffers.len() == 1)
        .and_then(|buffers| buffers[0].get("uri"))
        .and_then(Value::as_str)
        .ok_or("GLTF must have exactly one external buffer")?
        .to_owned();
    let mut binary = std::fs::read(confined_dependency(root, &buffer_uri)?)
        .map_err(|error| format!("read GLTF buffer {buffer_uri}: {error}"))?;
    let mut views = object
        .remove("bufferViews")
        .unwrap_or_else(|| Value::Array(Vec::new()))
        .as_array()
        .cloned()
        .ok_or("GLTF bufferViews must be an array")?;
    let mut images = object
        .remove("images")
        .unwrap_or_else(|| Value::Array(Vec::new()))
        .as_array()
        .cloned()
        .ok_or("GLTF images must be an array")?;
    for (index, image) in images.iter_mut().enumerate() {
        let image = image
            .as_object_mut()
            .ok_or_else(|| format!("GLTF image {index} must be an object"))?;
        let uri = image
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("GLTF image {index} has no external URI"))?
            .to_owned();
        while binary.len() % 4 != 0 {
            binary.push(0);
        }
        let offset = binary.len();
        let bytes = std::fs::read(confined_dependency(root, &uri)?)
            .map_err(|error| format!("read GLTF image {uri}: {error}"))?;
        binary.extend_from_slice(&bytes);
        let mut view = Map::new();
        view.insert("buffer".into(), Value::from(0));
        view.insert("byteOffset".into(), Value::from(offset));
        view.insert("byteLength".into(), Value::from(bytes.len()));
        let view_index = views.len();
        views.push(Value::Object(view));
        image.remove("uri");
        image.insert("bufferView".into(), Value::from(view_index));
        image.insert("mimeType".into(), Value::from(mime_for_uri(&uri)?));
    }
    object.insert("bufferViews".into(), Value::Array(views));
    object.insert("images".into(), Value::Array(images));
    let buffer = object
        .get_mut("buffers")
        .and_then(Value::as_array_mut)
        .and_then(|buffers| buffers[0].as_object_mut())
        .ok_or("GLTF buffer must be an object")?;
    buffer.remove("uri");
    buffer.insert("byteLength".into(), Value::from(binary.len()));
    let mut json =
        serde_json::to_vec(&document).map_err(|error| format!("serialize cooked GLTF: {error}"))?;
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    while binary.len() % 4 != 0 {
        binary.push(0);
    }
    let total = 12usize
        .checked_add(8 + json.len())
        .and_then(|n| n.checked_add(8 + binary.len()))
        .ok_or("cooked GLB size overflow")?;
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(&JSON_CHUNK.to_le_bytes());
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    glb.extend_from_slice(&BIN_CHUNK.to_le_bytes());
    glb.extend_from_slice(&binary);
    Ok(glb)
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "truncated GLB integer".to_owned())?
        .try_into()
        .unwrap();
    Ok(u32::from_le_bytes(raw))
}

/// Extract embedded images from a self-contained GLB without decoding meshes.
/// External image URIs are rejected: a cooked model must be independently
/// deployable and cannot retain side-file dependencies.
pub fn extract_glb_images(bytes: &[u8]) -> Result<Vec<GlbImage>, String> {
    if bytes.len() < 12 || u32_at(bytes, 0)? != GLB_MAGIC {
        return Err("invalid GLB magic".into());
    }
    if u32_at(bytes, 4)? != 2 {
        return Err("only GLB version 2 is supported".into());
    }
    let declared = u32_at(bytes, 8)? as usize;
    if declared != bytes.len() {
        return Err("GLB declared length does not match payload".into());
    }

    let mut json = None;
    let mut binary = None;
    let mut offset = 12usize;
    while offset < bytes.len() {
        let length = u32_at(bytes, offset)? as usize;
        let kind = u32_at(bytes, offset + 4)?;
        offset = offset.checked_add(8).ok_or("GLB chunk offset overflow")?;
        let end = offset
            .checked_add(length)
            .ok_or("GLB chunk length overflow")?;
        let chunk = bytes.get(offset..end).ok_or("truncated GLB chunk")?;
        match kind {
            JSON_CHUNK => {
                if json.replace(chunk).is_some() {
                    return Err("GLB contains multiple JSON chunks".into());
                }
            }
            BIN_CHUNK => {
                if binary.replace(chunk).is_some() {
                    return Err("GLB contains multiple BIN chunks".into());
                }
            }
            _ => {}
        }
        offset = end;
    }
    let document: GlbJson = serde_json::from_slice(json.ok_or("GLB has no JSON chunk")?)
        .map_err(|error| format!("invalid GLB JSON: {error}"))?;
    let binary = binary.ok_or("GLB has no BIN chunk")?;
    let mut result = Vec::with_capacity(document.images.len());
    for (index, image) in document.images.into_iter().enumerate() {
        if image.uri.is_some() {
            return Err(format!("GLB image {index} uses an external URI"));
        }
        let view_index = image
            .buffer_view
            .ok_or_else(|| format!("GLB image {index} has no bufferView"))?;
        let view = document.buffer_views.get(view_index).ok_or_else(|| {
            format!("GLB image {index} references missing bufferView {view_index}")
        })?;
        if view.buffer != 0 {
            return Err(format!(
                "GLB image {index} references non-GLB buffer {}",
                view.buffer
            ));
        }
        let end = view
            .byte_offset
            .checked_add(view.byte_length)
            .ok_or("GLB image range overflow")?;
        let data = binary
            .get(view.byte_offset..end)
            .ok_or_else(|| format!("GLB image {index} exceeds BIN chunk"))?;
        result.push(GlbImage {
            index,
            mime_type: image
                .mime_type
                .ok_or_else(|| format!("GLB image {index} has no mimeType"))?,
            bytes: data.to_vec(),
        });
    }
    Ok(result)
}

fn remap_buffer_view_references(
    value: &mut Value,
    mapping: &[Option<usize>],
) -> Result<(), String> {
    match value {
        Value::Array(values) => {
            for value in values {
                remap_buffer_view_references(value, mapping)?;
            }
        }
        Value::Object(object) => {
            if let Some(index) = object.get("bufferView").and_then(Value::as_u64) {
                let mapped = mapping
                    .get(index as usize)
                    .and_then(|entry| *entry)
                    .ok_or("GLB content references a stripped image bufferView")?;
                object.insert("bufferView".into(), Value::from(mapped));
            }
            for value in object.values_mut() {
                remap_buffer_view_references(value, mapping)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Remove browser-decodable image payloads from a runtime GLB after they have
/// been cooked into VT pages. Texture/material sampling metadata is retained in
/// an ignored custom extension for the web runtime's stable-index binding.
pub fn strip_glb_images_for_virtual_texturing(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 12 || u32_at(bytes, 0)? != GLB_MAGIC || u32_at(bytes, 4)? != 2 {
        return Err("invalid GLB header".into());
    }
    if u32_at(bytes, 8)? as usize != bytes.len() {
        return Err("GLB declared length does not match payload".into());
    }
    let mut json_chunk = None;
    let mut binary_chunk = None;
    let mut cursor = 12usize;
    while cursor < bytes.len() {
        let length = u32_at(bytes, cursor)? as usize;
        let kind = u32_at(bytes, cursor + 4)?;
        cursor = cursor.checked_add(8).ok_or("GLB chunk offset overflow")?;
        let end = cursor
            .checked_add(length)
            .ok_or("GLB chunk length overflow")?;
        let chunk = bytes.get(cursor..end).ok_or("truncated GLB chunk")?;
        if kind == JSON_CHUNK {
            json_chunk = Some(chunk);
        }
        if kind == BIN_CHUNK {
            binary_chunk = Some(chunk);
        }
        cursor = end;
    }
    let mut document: Value = serde_json::from_slice(json_chunk.ok_or("GLB has no JSON chunk")?)
        .map_err(|error| format!("invalid GLB JSON: {error}"))?;
    let binary = binary_chunk.ok_or("GLB has no BIN chunk")?;
    let object = document
        .as_object_mut()
        .ok_or("GLB JSON must be an object")?;
    if let Some(materials) = object.get("materials").and_then(Value::as_array) {
        for (index, material) in materials.iter().enumerate() {
            let material = material
                .as_object()
                .ok_or_else(|| format!("GLB material {index} must be an object"))?;
            if material.contains_key("occlusionTexture") {
                return Err(format!(
                    "GLB material {index} uses unsupported occlusionTexture"
                ));
            }
            if let Some(extensions) = material.get("extensions").and_then(Value::as_object) {
                for (name, extension) in extensions {
                    if name != "KHR_materials_transmission" {
                        return Err(format!(
                            "GLB material {index} uses unsupported material extension {name}"
                        ));
                    }
                    if extension.get("transmissionTexture").is_some() {
                        return Err(format!(
                            "GLB material {index} uses unsupported transmissionTexture"
                        ));
                    }
                }
            }
            let pbr = material
                .get("pbrMetallicRoughness")
                .and_then(Value::as_object);
            let has_base = pbr.is_some_and(|value| value.contains_key("baseColorTexture"));
            let has_other = pbr.is_some_and(|value| value.contains_key("metallicRoughnessTexture"))
                || material.contains_key("normalTexture")
                || material.contains_key("emissiveTexture");
            if has_other && !has_base {
                return Err(format!(
                    "GLB material {index} has virtual channels without base color"
                ));
            }
        }
    }
    let image_views: HashSet<usize> = object
        .get("images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|image| image.get("bufferView").and_then(Value::as_u64))
        .map(|index| index as usize)
        .collect();

    let mut metadata = Map::new();
    metadata.insert(
        "textures".into(),
        object
            .get("textures")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    );
    metadata.insert(
        "samplers".into(),
        object
            .get("samplers")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    );
    metadata.insert(
        "materials".into(),
        object
            .get("materials")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    );
    let extensions = object
        .entry("extensions")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or("GLB extensions must be an object")?;
    extensions.insert("AFTERGLOW_virtual_textures".into(), Value::Object(metadata));
    let used = object
        .entry("extensionsUsed")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("GLB extensionsUsed must be an array")?;
    if !used
        .iter()
        .any(|value| value.as_str() == Some("AFTERGLOW_virtual_textures"))
    {
        used.push(Value::from("AFTERGLOW_virtual_textures"));
    }

    if let Some(materials) = object.get_mut("materials").and_then(Value::as_array_mut) {
        for material in materials {
            let Some(material) = material.as_object_mut() else {
                continue;
            };
            if let Some(pbr) = material
                .get_mut("pbrMetallicRoughness")
                .and_then(Value::as_object_mut)
            {
                pbr.remove("baseColorTexture");
                pbr.remove("metallicRoughnessTexture");
            }
            material.remove("normalTexture");
            material.remove("occlusionTexture");
            material.remove("emissiveTexture");
        }
    }
    object.insert("images".into(), Value::Array(Vec::new()));
    object.insert("textures".into(), Value::Array(Vec::new()));
    object.insert("samplers".into(), Value::Array(Vec::new()));

    let view_count = object
        .get("bufferViews")
        .and_then(Value::as_array)
        .ok_or("GLB bufferViews must be an array")?
        .len();
    let mut mapping = vec![None; view_count];
    let mut retained_count = 0usize;
    for (index, target) in mapping.iter_mut().enumerate() {
        if !image_views.contains(&index) {
            *target = Some(retained_count);
            retained_count += 1;
        }
    }
    remap_buffer_view_references(&mut document, &mapping)?;
    let object = document
        .as_object_mut()
        .ok_or("GLB JSON must be an object")?;
    let views = object
        .remove("bufferViews")
        .and_then(|views| views.as_array().cloned())
        .ok_or("GLB bufferViews must be an array")?;
    let mut retained_views = Vec::with_capacity(retained_count);
    let mut compact = Vec::new();
    for (index, mut view) in views.into_iter().enumerate() {
        if image_views.contains(&index) {
            continue;
        }
        let view_object = view
            .as_object_mut()
            .ok_or("GLB bufferView must be an object")?;
        let offset = view_object
            .get("byteOffset")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let length = view_object
            .get("byteLength")
            .and_then(Value::as_u64)
            .ok_or("GLB bufferView has no byteLength")? as usize;
        while compact.len() % 4 != 0 {
            compact.push(0);
        }
        view_object.insert("byteOffset".into(), Value::from(compact.len()));
        let end = offset
            .checked_add(length)
            .ok_or("GLB bufferView range overflow")?;
        compact.extend_from_slice(
            binary
                .get(offset..end)
                .ok_or("GLB bufferView exceeds BIN chunk")?,
        );
        retained_views.push(view);
    }
    object.insert("bufferViews".into(), Value::Array(retained_views));
    let buffer = object
        .get_mut("buffers")
        .and_then(Value::as_array_mut)
        .and_then(|buffers| buffers.first_mut())
        .and_then(Value::as_object_mut)
        .ok_or("GLB buffer must be an object")?;
    buffer.insert("byteLength".into(), Value::from(compact.len()));

    let mut json = serde_json::to_vec(&document)
        .map_err(|error| format!("serialize stripped GLB: {error}"))?;
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    while compact.len() % 4 != 0 {
        compact.push(0);
    }
    let total = 12usize
        .checked_add(8 + json.len())
        .and_then(|n| n.checked_add(8 + compact.len()))
        .ok_or("stripped GLB size overflow")?;
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    output.extend_from_slice(&2u32.to_le_bytes());
    output.extend_from_slice(&(total as u32).to_le_bytes());
    output.extend_from_slice(&(json.len() as u32).to_le_bytes());
    output.extend_from_slice(&JSON_CHUNK.to_le_bytes());
    output.extend_from_slice(&json);
    output.extend_from_slice(&(compact.len() as u32).to_le_bytes());
    output.extend_from_slice(&BIN_CHUNK.to_le_bytes());
    output.extend_from_slice(&compact);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glb(json: &str, binary: &[u8]) -> Vec<u8> {
        let mut json = json.as_bytes().to_vec();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let mut bin = binary.to_vec();
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let length = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = Vec::new();
        out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(length as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&JSON_CHUNK.to_le_bytes());
        out.extend_from_slice(&json);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&BIN_CHUNK.to_le_bytes());
        out.extend_from_slice(&bin);
        out
    }

    #[test]
    fn embeds_external_gltf_buffer_and_image() {
        let root = std::env::temp_dir().join(format!("afterglow-gltf-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("textures")).unwrap();
        std::fs::write(root.join("scene.bin"), [1, 2, 3, 4]).unwrap();
        std::fs::write(root.join("textures/color.png"), [9, 8, 7]).unwrap();
        std::fs::write(root.join("scene.gltf"), r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"scene.bin","byteLength":4}],"images":[{"uri":"textures/color.png"}]}"#).unwrap();
        let packed = embed_external_gltf(&root.join("scene.gltf")).unwrap();
        let images = extract_glb_images(&packed).unwrap();
        assert_eq!(images[0].mime_type, "image/png");
        assert_eq!(images[0].bytes, vec![9, 8, 7]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn extracts_embedded_images_by_buffer_view() {
        let bytes = glb(
            r#"{"bufferViews":[{"buffer":0,"byteOffset":2,"byteLength":3}],"images":[{"bufferView":0,"mimeType":"image/png"}]}"#,
            &[9, 8, 1, 2, 3],
        );
        let images = extract_glb_images(&bytes).unwrap();
        assert_eq!(
            images,
            vec![GlbImage {
                index: 0,
                mime_type: "image/png".into(),
                bytes: vec![1, 2, 3]
            }]
        );
    }

    #[test]
    fn strips_runtime_images_and_retains_virtual_texture_metadata() {
        let source = glb(
            r#"{"buffers":[{"byteLength":8}],"bufferViews":[{"buffer":0,"byteOffset":4,"byteLength":4},{"buffer":0,"byteOffset":0,"byteLength":4}],"accessors":[{"bufferView":1,"componentType":5121,"count":4,"type":"SCALAR"}],"images":[{"bufferView":0,"mimeType":"image/png"}],"textures":[{"source":0,"sampler":0}],"samplers":[{"wrapS":33648,"wrapT":33648}],"materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0,"extensions":{"KHR_texture_transform":{"offset":[0.2,0.3]}}}},"extensions":{"KHR_materials_transmission":{"transmissionFactor":0.35}}}]}"#,
            &[1, 2, 3, 4, 90, 91, 92, 93],
        );
        let stripped = strip_glb_images_for_virtual_texturing(&source).unwrap();
        assert!(extract_glb_images(&stripped).unwrap().is_empty());
        assert!(!stripped.windows(4).any(|window| window == [90, 91, 92, 93]));
        let json_len = u32_at(&stripped, 12).unwrap() as usize;
        let document: Value = serde_json::from_slice(&stripped[20..20 + json_len]).unwrap();
        assert_eq!(document["images"].as_array().unwrap().len(), 0);
        assert!(document["materials"][0]["pbrMetallicRoughness"]
            .get("baseColorTexture")
            .is_none());
        assert_eq!(
            document["extensions"]["AFTERGLOW_virtual_textures"]["materials"][0]
                ["pbrMetallicRoughness"]["baseColorTexture"]["index"],
            0,
        );
        assert_eq!(
            document["materials"][0]["extensions"]["KHR_materials_transmission"]
                ["transmissionFactor"],
            0.35,
        );
        assert_eq!(document["bufferViews"].as_array().unwrap().len(), 1);
        assert_eq!(document["accessors"][0]["bufferView"], 0);
        let bin_offset = 20 + json_len + 8;
        assert_eq!(&stripped[bin_offset..bin_offset + 4], &[1, 2, 3, 4]);
    }

    #[test]
    fn rejects_material_channels_the_runtime_binding_cannot_preserve() {
        let unsupported = glb(
            r#"{"buffers":[{"byteLength":4}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":4}],"images":[{"bufferView":0,"mimeType":"image/png"}],"textures":[{"source":0}],"materials":[{"occlusionTexture":{"index":0}}]}"#,
            &[1, 2, 3, 4],
        );
        assert!(strip_glb_images_for_virtual_texturing(&unsupported)
            .unwrap_err()
            .contains("unsupported occlusionTexture"));
    }

    #[test]
    fn rejects_external_or_out_of_bounds_images() {
        let external = glb(r#"{"images":[{"uri":"texture.png"}]}"#, &[]);
        assert!(extract_glb_images(&external)
            .unwrap_err()
            .contains("external URI"));
        let truncated = glb(
            r#"{"bufferViews":[{"buffer":0,"byteOffset":3,"byteLength":8}],"images":[{"bufferView":0,"mimeType":"image/png"}]}"#,
            &[1, 2, 3, 4],
        );
        assert!(extract_glb_images(&truncated)
            .unwrap_err()
            .contains("exceeds BIN"));
    }
}
