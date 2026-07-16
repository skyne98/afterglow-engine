use serde::Deserialize;
use serde_json::{Map, Value};
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
    fn rejects_external_or_out_of_bounds_images() {
        let external = glb(r#"{"images":[{"uri":"texture.png"}]}"#, &[]);
        assert!(
            extract_glb_images(&external)
                .unwrap_err()
                .contains("external URI")
        );
        let truncated = glb(
            r#"{"bufferViews":[{"buffer":0,"byteOffset":3,"byteLength":8}],"images":[{"bufferView":0,"mimeType":"image/png"}]}"#,
            &[1, 2, 3, 4],
        );
        assert!(
            extract_glb_images(&truncated)
                .unwrap_err()
                .contains("exceeds BIN")
        );
    }
}
