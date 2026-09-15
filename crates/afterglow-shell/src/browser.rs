mod canvas;
pub use canvas::RasterRegion;
use canvas::CanvasPixels;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use blitz_traits::NodeId;

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{
    Attribute, BaseDocument, DocumentConfig, LocalName, Namespace, Prefix, QualName,
    build_browser_font_ctx,
};
use blitz_paint::{paint_scene, paint_scene_region};
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TextInputAction {
    pub action: String,
    pub key: String,
    pub x: f32,
    pub y: f32,
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
    pub anchor: usize,
    pub focus: usize,
    pub cursor: Option<(usize, usize)>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextInputState {
    pub value: String,
    pub anchor: usize,
    pub focus: usize,
    pub cursor_rect: Option<DomRect>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl DomRect {
    fn from_parts(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
            top: y,
            right: x + width,
            bottom: y + height,
            left: x,
        }
    }
}

fn intersect_rect(
    a: blitz_dom::BoundingRect,
    b: blitz_dom::BoundingRect,
) -> blitz_dom::BoundingRect {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width).max(left);
    let bottom = (a.y + a.height).min(b.y + b.height).max(top);
    blitz_dom::BoundingRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

fn intersect_x(
    mut a: blitz_dom::BoundingRect,
    b: blitz_dom::BoundingRect,
) -> blitz_dom::BoundingRect {
    let left = a.x.max(b.x);
    let right = (a.x + a.width).min(b.x + b.width).max(left);
    a.x = left;
    a.width = right - left;
    a
}

fn intersect_y(
    mut a: blitz_dom::BoundingRect,
    b: blitz_dom::BoundingRect,
) -> blitz_dom::BoundingRect {
    let top = a.y.max(b.y);
    let bottom = (a.y + a.height).min(b.y + b.height).max(top);
    a.y = top;
    a.height = bottom - top;
    a
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomIntersection {
    pub intersection_rect: DomRect,
    pub root_bounds: DomRect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomBoxMetrics {
    pub client_width: i32,
    pub client_height: i32,
    pub client_left: i32,
    pub client_top: i32,
    pub offset_width: i32,
    pub offset_height: i32,
    pub offset_left: i32,
    pub offset_top: i32,
    pub offset_parent: Option<u64>,
    pub scroll_width: i32,
    pub scroll_height: i32,
    pub scroll_left: f64,
    pub scroll_top: f64,
}

#[derive(Default)]
struct BrowserNetProvider;

impl NetProvider for BrowserNetProvider {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let resolved_url = request.url.to_string();
        let bytes = match request.url.scheme() {
            "file" => {
                let path = request.url.to_file_path().unwrap_or_else(|_| {
                    panic!("invalid file URL requested by Blitz: {resolved_url}")
                });
                std::fs::read(&path).unwrap_or_else(|error| {
                    panic!("read Blitz resource {}: {error}", path.display())
                })
            }
            "data" => {
                data_url::DataUrl::process(request.url.as_str())
                    .unwrap_or_else(|error| panic!("parse Blitz data URL: {error}"))
                    .decode_to_vec()
                    .unwrap_or_else(|error| panic!("decode Blitz data URL: {error:?}"))
                    .0
            }
            scheme => panic!("unsupported Blitz resource scheme {scheme:?}: {resolved_url}"),
        };
        handler.bytes(resolved_url, Bytes::from(bytes));
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSnapshot {
    pub nodes: Vec<BrowserNodeRecord>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserNodeRecord {
    pub id: u64,
    pub kind: String,
    pub local_name: Option<String>,
    pub namespace: Option<String>,
    pub prefix: Option<String>,
    pub attributes: Vec<BrowserAttributeRecord>,
    pub text: Option<String>,
    #[serde(default)]
    pub stylesheet_text: Option<String>,
    #[serde(default)]
    pub checked: Option<bool>,
    pub children: Vec<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAttributeRecord {
    pub local_name: String,
    pub namespace: Option<String>,
    pub prefix: Option<String>,
    pub value: String,
}

pub struct CanvasRaster {
    pub native_id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Native style/layout/paint state mirrored from the JavaScript-facing
/// LinkeDOM tree through stable out-of-band IDs and structured node records.
/// Reconciliation preserves the long-lived Blitz document and derived state.
pub struct BrowserDocument {
    document: Option<BaseDocument>,
    nodes: HashMap<u64, NodeId>,
    stylesheet_texts: HashMap<u64, String>,
    canvas_pixels: HashMap<u64, CanvasPixels>,
    viewport_width: u32,
    viewport_height: u32,
    raster_width: u32,
    raster_height: u32,
    viewport_scale: f64,
    epoch: u64,
    animation_clock: Instant,
}

impl BrowserDocument {
    pub fn new(viewport_width: u32, viewport_height: u32) -> Self {
        Self {
            document: None,
            nodes: HashMap::new(),
            stylesheet_texts: HashMap::new(),
            canvas_pixels: HashMap::new(),
            viewport_width,
            viewport_height,
            raster_width: viewport_width,
            raster_height: viewport_height,
            viewport_scale: 1.0,
            epoch: 0,
            animation_clock: Instant::now(),
        }
    }

    pub fn is_animating(&self) -> bool {
        self.document.as_ref().is_some_and(BaseDocument::is_animating)
    }

    pub fn advance_animations(&mut self) {
        if let Some(document) = self.document.as_mut() {
            if document.is_animating() {
                document.resolve(self.animation_clock.elapsed().as_secs_f64());
            }
        }
    }

    pub fn sync(
        &mut self,
        epoch: u64,
        snapshot: BrowserSnapshot,
        base_url: &str,
    ) -> Result<(), String> {
        if self.document.is_some() && self.epoch == epoch {
            return Ok(());
        }
        if self.document.is_some() {
            return self.reconcile(epoch, snapshot, base_url);
        }

        if snapshot.nodes.is_empty() {
            return Err("LinkeDOM snapshot contains no nodes".to_string());
        }
        let mut document = BaseDocument::new(DocumentConfig {
            base_url: Some(base_url.to_string()),
            viewport: Some(Viewport::new(
                self.viewport_width,
                self.viewport_height,
                1.0,
                ColorScheme::Light,
            )),
            font_ctx: Some(build_browser_font_ctx(
                include_bytes!("../vendor/fonts/LiberationSans-Regular.ttf"),
                include_bytes!("../vendor/fonts/JetBrainsMonoNerdFontMono-Regular.ttf"),
            )),
            net_provider: Some(Arc::new(BrowserNetProvider)),
            ..Default::default()
        });

        let mut nodes = HashMap::with_capacity(snapshot.nodes.len());
        let mut referenced_children = std::collections::HashSet::new();
        {
            let mut mutator = document.mutate();
            for record in &snapshot.nodes {
                if record.id == 0 || nodes.contains_key(&record.id) {
                    return Err(format!("invalid or duplicate native node ID {}", record.id));
                }
                let node_id = match record.kind.as_str() {
                    "element" => {
                        let local_name = record
                            .local_name
                            .as_deref()
                            .ok_or_else(|| format!("element {} has no local name", record.id))?;
                        let attributes = record
                            .attributes
                            .iter()
                            .map(|attribute| Attribute {
                                name: QualName::new(
                                    attribute.prefix.as_deref().map(Prefix::from),
                                    Namespace::from(attribute.namespace.as_deref().unwrap_or("")),
                                    LocalName::from(attribute.local_name.as_str()),
                                ),
                                value: attribute.value.clone(),
                            })
                            .collect();
                        mutator.create_element(
                            QualName::new(
                                record.prefix.as_deref().map(Prefix::from),
                                Namespace::from(record.namespace.as_deref().unwrap_or("")),
                                LocalName::from(local_name),
                            ),
                            attributes,
                        )
                    }
                    "text" => mutator.create_text_node(record.text.as_deref().unwrap_or("")),
                    "comment" => mutator.create_comment_node(record.text.as_deref().unwrap_or("")),
                    kind => {
                        return Err(format!(
                            "native node {} has unknown kind {kind:?}",
                            record.id
                        ));
                    }
                };
                nodes.insert(record.id, node_id);
                for &child in &record.children {
                    if !referenced_children.insert(child) {
                        return Err(format!("native node {child} has multiple parents"));
                    }
                }
            }

            let roots = snapshot
                .nodes
                .iter()
                .filter(|record| !referenced_children.contains(&record.id))
                .map(|record| nodes[&record.id])
                .collect::<Vec<_>>();
            if roots.len() != 1 {
                return Err(format!(
                    "LinkeDOM snapshot has {} roots, expected one",
                    roots.len()
                ));
            }
            let document_root = mutator.doc.root_node().id;
            mutator.append_children(document_root, &roots);
            for record in &snapshot.nodes {
                let children = record
                    .children
                    .iter()
                    .map(|child| {
                        nodes.get(child).copied().ok_or_else(|| {
                            format!("node {} references missing child {child}", record.id)
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if !children.is_empty() {
                    mutator.append_children(nodes[&record.id], &children);
                }
            }
        }
        for record in &snapshot.nodes {
            if let Some(css) = &record.stylesheet_text {
                document.set_stylesheet_text_for_node(nodes[&record.id], css);
                self.stylesheet_texts.insert(record.id, css.clone());
            }
        }

        // Attach a transparent first paint source while retaining canvas as a
        // replaced element. The local Blitz patch owns intrinsic canvas sizing.
        let canvas_sizes = nodes
            .values()
            .copied()
            .filter_map(|node_id| {
                let element = document.get_node(node_id)?.element_data()?;
                if element.name.local.as_ref() != "canvas" {
                    return None;
                }
                let width = element
                    .attr(LocalName::from("width"))
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(300)
                    .max(1);
                let height = element
                    .attr(LocalName::from("height"))
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(150)
                    .max(1);
                Some((node_id, width, height))
            })
            .collect::<Vec<_>>();
        {
            let mut mutator = document.mutate();
            for record in &snapshot.nodes {
                if let Some(checked) = record.checked {
                    mutator.set_input_checked(nodes[&record.id], checked)?;
                }
            }
            for (node_id, width, height) in canvas_sizes {
                mutator
                    .set_canvas_raster(
                        node_id,
                        width,
                        height,
                        Arc::new(vec![0; width as usize * height as usize * 4]),
                    )
                    .map_err(|error| format!("set Blitz canvas {node_id}: {error}"))?;
            }
        }
        document.resolve(self.animation_clock.elapsed().as_secs_f64());
        self.document = Some(document);
        self.nodes = nodes;
        self.epoch = epoch;
        Ok(())
    }

    fn reconcile(
        &mut self,
        epoch: u64,
        snapshot: BrowserSnapshot,
        base_url: &str,
    ) -> Result<(), String> {
        if snapshot.nodes.is_empty() {
            return Err("LinkeDOM snapshot contains no nodes".to_string());
        }
        let document = self.document.as_mut().unwrap();
        document.set_base_url(base_url);
        let snapshot_ids = snapshot
            .nodes
            .iter()
            .map(|record| record.id)
            .collect::<std::collections::HashSet<_>>();
        if snapshot_ids.len() != snapshot.nodes.len() || snapshot_ids.contains(&0) {
            return Err("LinkeDOM snapshot contains invalid or duplicate IDs".to_string());
        }
        let stale_native_ids = self
            .nodes
            .keys()
            .filter(|id| !snapshot_ids.contains(id))
            .copied()
            .collect::<Vec<_>>();
        let mut referenced_children = std::collections::HashSet::new();
        for record in &snapshot.nodes {
            for child in &record.children {
                if !snapshot_ids.contains(child) {
                    return Err(format!(
                        "node {} references missing child {child}",
                        record.id
                    ));
                }
                if !referenced_children.insert(*child) {
                    return Err(format!("native node {child} has multiple parents"));
                }
            }
        }
        let roots = snapshot
            .nodes
            .iter()
            .filter(|record| !referenced_children.contains(&record.id))
            .map(|record| record.id)
            .collect::<Vec<_>>();
        if roots.len() != 1 {
            return Err(format!(
                "LinkeDOM snapshot has {} roots, expected one",
                roots.len()
            ));
        }

        {
            let mut mutator = document.mutate();
            for record in &snapshot.nodes {
                if self.nodes.contains_key(&record.id) {
                    continue;
                }
                let node_id = match record.kind.as_str() {
                    "element" => {
                        let local_name = record
                            .local_name
                            .as_deref()
                            .ok_or_else(|| format!("element {} has no local name", record.id))?;
                        let attributes = record
                            .attributes
                            .iter()
                            .map(|attribute| Attribute {
                                name: QualName::new(
                                    attribute.prefix.as_deref().map(Prefix::from),
                                    Namespace::from(attribute.namespace.as_deref().unwrap_or("")),
                                    LocalName::from(attribute.local_name.as_str()),
                                ),
                                value: attribute.value.clone(),
                            })
                            .collect();
                        mutator.create_element(
                            QualName::new(
                                record.prefix.as_deref().map(Prefix::from),
                                Namespace::from(record.namespace.as_deref().unwrap_or("")),
                                LocalName::from(local_name),
                            ),
                            attributes,
                        )
                    }
                    "text" => mutator.create_text_node(record.text.as_deref().unwrap_or("")),
                    "comment" => mutator.create_comment_node(record.text.as_deref().unwrap_or("")),
                    kind => {
                        return Err(format!(
                            "native node {} has unknown kind {kind:?}",
                            record.id
                        ));
                    }
                };
                self.nodes.insert(record.id, node_id);
            }

            for record in &snapshot.nodes {
                let node_id = self.nodes[&record.id];
                match record.kind.as_str() {
                    "element" => {
                        let element = mutator
                            .doc
                            .get_node(node_id)
                            .and_then(|node| node.element_data())
                            .ok_or_else(|| {
                                format!("native element {} changed node kind", record.id)
                            })?;
                        if element.name.local.as_ref()
                            != record.local_name.as_deref().unwrap_or_default()
                            || element.name.ns.as_ref() != record.namespace.as_deref().unwrap_or("")
                        {
                            return Err(format!(
                                "native element {} changed qualified name",
                                record.id
                            ));
                        }
                        let current = element.attrs().to_vec();
                        for attribute in current {
                            let wanted = record.attributes.iter().find(|wanted| {
                                attribute.name.local.as_ref() == wanted.local_name
                                    && attribute.name.ns.as_ref()
                                        == wanted.namespace.as_deref().unwrap_or("")
                                    && attribute.name.prefix.as_ref().map(|p| p.as_ref())
                                        == wanted.prefix.as_deref()
                            });
                            if wanted.is_none() {
                                mutator.clear_attribute(node_id, attribute.name);
                            }
                        }
                        for attribute in &record.attributes {
                            let name = QualName::new(
                                attribute.prefix.as_deref().map(Prefix::from),
                                Namespace::from(attribute.namespace.as_deref().unwrap_or("")),
                                LocalName::from(attribute.local_name.as_str()),
                            );
                            let unchanged = mutator
                                .doc
                                .get_node(node_id)
                                .and_then(|node| node.element_data())
                                .and_then(|element| {
                                    element.attrs().iter().find(|current| current.name == name)
                                })
                                .is_some_and(|current| current.value == attribute.value);
                            if !unchanged {
                                mutator.set_attribute(node_id, name, &attribute.value);
                            }
                        }
                    }
                    "text" => mutator.set_node_text(node_id, record.text.as_deref().unwrap_or("")),
                    "comment" => {}
                    kind => {
                        return Err(format!(
                            "native node {} has unknown kind {kind:?}",
                            record.id
                        ));
                    }
                }
            }

            let document_root = mutator.doc.root_node().id;
            let mut desired_trees = vec![(document_root, vec![self.nodes[&roots[0]]])];
            desired_trees.extend(snapshot.nodes.iter().map(|record| {
                (
                    self.nodes[&record.id],
                    record
                        .children
                        .iter()
                        .map(|child| self.nodes[child])
                        .collect(),
                )
            }));
            for (parent, desired) in desired_trees {
                if mutator.child_ids(parent) == desired {
                    continue;
                }
                for child in mutator.child_ids(parent) {
                    mutator.remove_node(child);
                }
                if !desired.is_empty() {
                    mutator.append_children(parent, &desired);
                }
            }
            let stale_roots = stale_native_ids
                .iter()
                .filter_map(|native_id| self.nodes.get(native_id).copied())
                .filter(|node_id| {
                    mutator
                        .doc
                        .get_node(*node_id)
                        .is_some_and(|node| node.parent.is_none())
                })
                .collect::<Vec<_>>();
            for node_id in stale_roots {
                mutator.remove_and_drop_node(node_id);
            }
        }
        self.nodes
            .retain(|native_id, _| snapshot_ids.contains(native_id));
        self.stylesheet_texts
            .retain(|id, _| snapshot_ids.contains(id));
        for record in &snapshot.nodes {
            if let Some(css) = &record.stylesheet_text {
                if self.stylesheet_texts.get(&record.id) != Some(css) {
                    document.set_stylesheet_text_for_node(self.nodes[&record.id], css);
                    self.stylesheet_texts.insert(record.id, css.clone());
                }
            } else {
                self.stylesheet_texts.remove(&record.id);
            }
        }

        let canvases = snapshot
            .nodes
            .iter()
            .filter(|record| record.local_name.as_deref() == Some("canvas"))
            .map(|record| self.nodes[&record.id])
            .collect::<Vec<_>>();
        {
            let mut mutator = document.mutate();
            for record in &snapshot.nodes {
                if let Some(checked) = record.checked {
                    mutator.set_input_checked(self.nodes[&record.id], checked)?;
                }
            }
            for node_id in canvases {
                let element = mutator
                    .doc
                    .get_node(node_id)
                    .and_then(|node| node.element_data())
                    .ok_or_else(|| format!("missing Blitz canvas node {node_id}"))?;
                if element.raster_image_data().is_some() {
                    continue;
                }
                let width = element
                    .attr(LocalName::from("width"))
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(300)
                    .max(1);
                let height = element
                    .attr(LocalName::from("height"))
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or(150)
                    .max(1);
                mutator
                    .set_canvas_raster(
                        node_id,
                        width,
                        height,
                        Arc::new(vec![0; width as usize * height as usize * 4]),
                    )
                    .map_err(|error| format!("set Blitz canvas {node_id}: {error}"))?;
            }
        }
        document.resolve(self.animation_clock.elapsed().as_secs_f64());
        self.epoch = epoch;
        Ok(())
    }

    pub fn computed_property(
        &self,
        native_id: u64,
        property_name: &str,
        pseudo: &str,
    ) -> Result<String, String> {
        let document = self
            .document
            .as_ref()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;
        let element_id = self
            .nodes
            .get(&native_id)
            .copied()
            .ok_or_else(|| format!("native node {native_id} is not connected"))?;
        let node_id = match pseudo {
            "" => Some(element_id),
            "::before" => document.get_node(element_id).and_then(|node| node.before()),
            "::after" => document.get_node(element_id).and_then(|node| node.after()),
            _ => return Err(format!("unsupported pseudo-element {pseudo:?}")),
        };
        let Some(node_id) = node_id else {
            return Ok(String::new());
        };
        Ok(document
            .computed_property(node_id, property_name)
            .unwrap_or_default())
    }

    pub fn media_query_matches(&self, query: &str) -> Result<bool, String> {
        let document = self
            .document
            .as_ref()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;
        Ok(document.media_query_matches(query))
    }

    pub fn set_focus(&mut self, native_id: Option<u64>) -> Result<bool, String> {
        let document = self
            .document
            .as_mut()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;
        let changed = match native_id {
            Some(native_id) => {
                let node_id = self
                    .nodes
                    .get(&native_id)
                    .copied()
                    .ok_or_else(|| format!("native node {native_id} is not connected"))?;
                document.set_focus_to(node_id)
            }
            None => {
                let had_focus = document.get_focussed_node_id().is_some();
                document.clear_focus();
                had_focus
            }
        };
        if changed {
            document.resolve(self.animation_clock.elapsed().as_secs_f64());
        }
        Ok(changed)
    }

    pub fn text_input(&mut self, native_id: u64, action: TextInputAction) -> Result<TextInputState, String> {
        use blitz_traits::events::*;
        use keyboard_types::{Code, Key, Location, Modifiers};
        let node_id = *self.nodes.get(&native_id).ok_or("text input is not connected")?;
        let document = self.document.as_mut().ok_or("browser document is missing")?;
        let element = document.get_node(node_id).and_then(|n| n.element_data()).ok_or("text input is missing")?;
        let input = element.text_input_data().ok_or("element has no text editor")?;
        let disabled = element.attr(LocalName::from("disabled")).is_some();
        let readonly = element.attr(LocalName::from("readonly")).is_some();
        let mut mods = Modifiers::empty();
        mods.set(Modifiers::SHIFT, action.shift);
        mods.set(Modifiers::CONTROL, action.control);
        mods.set(Modifiers::ALT, action.alt);
        mods.set(Modifiers::SUPER, action.meta);
        if action.key.len() > 32 * 1024 * 1024 { return Err("text input exceeds 32 MiB".into()); }
        let key = if action.action == "insert" { Key::Character(action.key.clone()) }
            else { action.key.parse::<Key>().unwrap_or(Key::Unidentified) };
        let selection_key = matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End)
            || (action.control || action.meta) && matches!(&key, Key::Character(c) if c.eq_ignore_ascii_case("a") || c.eq_ignore_ascii_case("c"));
        if action.action == "select" && !disabled {
            let text = input.editor.raw_text();
            let byte_offset = |offset: usize| {
                let mut units = 0;
                for (byte, character) in text.char_indices() {
                    if units >= offset { return byte; }
                    units += character.len_utf16();
                }
                text.len()
            };
            let anchor = byte_offset(action.anchor);
            let focus = byte_offset(action.focus);
            document.set_text_input_selection(node_id, anchor, focus);
        } else if !disabled && action.action != "query" {
            if action.action == "imeCommit" && !readonly {
                document.handle_dom_event(&mut DomEvent::new(node_id, DomEventData::Ime(BlitzImeEvent::Disabled)), |_| {});
            }
            let data = match action.action.as_str() {
                "imePreedit" if !readonly => {
                    if action.cursor.is_some_and(|(start, end)| !action.key.is_char_boundary(start) || !action.key.is_char_boundary(end)) {
                        return Err("invalid IME cursor byte offsets".into());
                    }
                    DomEventData::Ime(BlitzImeEvent::Preedit(action.key.into(), action.cursor))
                }
                "imeCommit" if !readonly => DomEventData::Ime(BlitzImeEvent::Commit(action.key.into())),
                "imeCancel" => DomEventData::Ime(BlitzImeEvent::Disabled),
                "key" | "insert" if !readonly || selection_key => DomEventData::KeyDown(BlitzKeyEvent {
                    key, code: Code::Unidentified, modifiers: mods, location: Location::Standard,
                    is_auto_repeating: false, is_composing: false, state: KeyState::Pressed, text: None,
                }),
                "down" | "move" | "up" => {
                    if !action.x.is_finite() || !action.y.is_finite() { return Err("invalid text pointer coordinates".into()); }
                    let pointer = BlitzPointerEvent {
                        id: BlitzPointerId::Mouse, is_primary: true,
                        coords: PointerCoords { page_x: action.x, page_y: action.y, screen_x: action.x, screen_y: action.y, client_x: action.x, client_y: action.y },
                        button: MouseEventButton::Main,
                        buttons: if action.action == "up" { MouseEventButtons::None } else { MouseEventButtons::Primary },
                        mods, details: Default::default(), element: Default::default(), active_pointers: Default::default(),
                    };
                    match action.action.as_str() { "down" => DomEventData::PointerDown(pointer), "move" => DomEventData::PointerMove(pointer), _ => DomEventData::PointerUp(pointer) }
                }
                "key" | "insert" | "imePreedit" | "imeCommit" => return self.text_input(native_id, TextInputAction { action: "query".into(), ..Default::default() }),
                _ => return Err("invalid text input action".into()),
            };
            document.handle_dom_event(&mut DomEvent::new(node_id, data), |_| {});
        }
        document.resolve(self.animation_clock.elapsed().as_secs_f64());
        let input = document.get_node(node_id).and_then(|n| n.element_data()).and_then(|el| el.text_input_data()).ok_or("text editor is missing")?;
        let text = input.editor.raw_text();
        let selection = input.editor.raw_selection();
        Ok(TextInputState {
            value: text.to_owned(),
            anchor: text[..selection.anchor().index()].encode_utf16().count(),
            focus: text[..selection.focus().index()].encode_utf16().count(),
            cursor_rect: document.text_input_cursor_rect(node_id).map(|rect| DomRect::from_parts(rect.x, rect.y, rect.width, rect.height)),
        })
    }

    fn cursor_for_node(&self, native_id: u64) -> Result<String, String> {
        let cursor = self.computed_property(native_id, "cursor", "")?;
        if cursor != "auto" {
            return Ok(cursor);
        }
        let node_id = self.nodes[&native_id];
        let element = self
            .document
            .as_ref()
            .and_then(|document| document.get_node(node_id))
            .and_then(|node| node.element_data());
        let Some(element) = element else {
            return Ok("default".to_string());
        };
        let local_name = element.name.local.as_ref();
        let input_type = element.attr(LocalName::from("type")).unwrap_or("text");
        let is_link = local_name == "a" && element.attr(LocalName::from("href")).is_some();
        let is_clickable_control = matches!(
            local_name,
            "button" | "select" | "option" | "summary" | "label"
        ) || local_name == "input"
            && matches!(
                input_type,
                "button"
                    | "checkbox"
                    | "color"
                    | "file"
                    | "image"
                    | "radio"
                    | "range"
                    | "reset"
                    | "submit"
            );
        if is_link || is_clickable_control {
            return Ok("pointer".to_string());
        }
        let is_editable = local_name == "textarea"
            || local_name == "input"
            || element
                .attr(LocalName::from("contenteditable"))
                .is_some_and(|value| value.is_empty() || value.eq_ignore_ascii_case("true"));
        let is_text_container = matches!(
            local_name,
            "abbr"
                | "address"
                | "article"
                | "b"
                | "blockquote"
                | "code"
                | "dd"
                | "del"
                | "dfn"
                | "dt"
                | "em"
                | "figcaption"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "i"
                | "ins"
                | "kbd"
                | "li"
                | "mark"
                | "p"
                | "pre"
                | "q"
                | "s"
                | "samp"
                | "small"
                | "span"
                | "strong"
                | "sub"
                | "sup"
                | "time"
                | "u"
                | "var"
        );
        Ok(if is_editable || is_text_container {
            "text"
        } else {
            "default"
        }
        .to_string())
    }

    pub fn cursor_at(&self, x: f64, y: f64) -> Result<String, String> {
        if let Some(native_id) = self.hit_test(x, y)? {
            let cursor = self.cursor_for_node(native_id)?;
            if cursor != "default" {
                return Ok(cursor);
            }
        }

        Ok("default".to_string())
    }

    pub fn set_pointer_state(&mut self, action: u32, x: f64, y: f64) -> Result<bool, String> {
        let document = self
            .document
            .as_mut()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;
        let hover_changed = if action <= 1 {
            document.set_hover_to(x as f32, y as f32)
        } else {
            false
        };
        let state_changed = match action {
            0 => false,
            1 => document.active_node(),
            2 => document.unactive_node(),
            _ => return Err(format!("invalid pointer state action {action}")),
        };
        if hover_changed || state_changed {
            document.resolve(self.animation_clock.elapsed().as_secs_f64());
        }
        Ok(hover_changed || state_changed)
    }

    pub fn set_scroll(&mut self, native_id: u64, left: f64, top: f64) -> Result<bool, String> {
        let node_id = self
            .nodes
            .get(&native_id)
            .copied()
            .ok_or_else(|| format!("native node {native_id} is not connected"))?;
        let document = self
            .document
            .as_mut()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;
        let current = document
            .get_node(node_id)
            .ok_or_else(|| format!("missing Blitz node {node_id}"))?
            .scroll_offset();
        let target_left = left.max(0.0);
        let target_top = top.max(0.0);
        Ok(document.scroll_node_by_has_changed(
            node_id,
            current.x - target_left,
            current.y - target_top,
            |_| {},
        ))
    }

    pub fn box_metrics(&self, native_id: u64) -> Result<DomBoxMetrics, String> {
        let document = self
            .document
            .as_ref()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;
        let node_id = self
            .nodes
            .get(&native_id)
            .copied()
            .ok_or_else(|| format!("native node {native_id} is not connected"))?;
        let node = document
            .get_node(node_id)
            .ok_or_else(|| format!("missing Blitz node {node_id}"))?;
        let layout = node.final_layout();
        let client_width = (layout.size.width - layout.border.left - layout.border.right).max(0.0);
        let client_height =
            (layout.size.height - layout.border.top - layout.border.bottom).max(0.0);
        let scroll_width = client_width.max(node.scrollable_overflow().x1 as f32);
        let scroll_height = client_height.max(node.scrollable_overflow().y1 as f32);

        let offset = node.offset_top_left();
        let offset_parent = node.offset_parent().and_then(|parent| {
            self.nodes
                .iter()
                .find_map(|(native, blitz)| (*blitz == parent.id).then_some(*native))
        });

        Ok(DomBoxMetrics {
            client_width: client_width.round() as i32,
            client_height: client_height.round() as i32,
            client_left: layout.border.left.round() as i32,
            client_top: layout.border.top.round() as i32,
            offset_width: layout.size.width.round() as i32,
            offset_height: layout.size.height.round() as i32,
            offset_left: offset.x.round() as i32,
            offset_top: offset.y.round() as i32,
            offset_parent,
            scroll_width: scroll_width.round() as i32,
            scroll_height: scroll_height.round() as i32,
            scroll_left: node.scroll_offset().x,
            scroll_top: node.scroll_offset().y,
        })
    }

    pub fn hit_tests(&self, x: f64, y: f64) -> Result<Vec<u64>, String> {
        let document = self
            .document
            .as_ref()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x >= self.viewport_width as f64
            || y >= self.viewport_height as f64
        {
            return Ok(Vec::new());
        }
        let reverse_nodes: HashMap<NodeId, u64> = self
            .nodes
            .iter()
            .map(|(native, blitz)| (*blitz, *native))
            .collect();
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        for hit in document.hits(x as f32, y as f32) {
            let mut node_id = Some(hit.node_id);
            while let Some(id) = node_id {
                if let Some(native_id) = reverse_nodes.get(&id).copied() {
                    if seen.insert(native_id) {
                        result.push(native_id);
                    }
                    break;
                }
                node_id = document.get_node(id).and_then(|node| node.parent);
            }
        }
        Ok(result)
    }

    pub fn intersection(
        &self,
        native_id: u64,
        root_native_id: Option<u64>,
        margins: [f64; 4],
    ) -> Result<DomIntersection, String> {
        let document = self
            .document
            .as_ref()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;
        let node_id = self
            .nodes
            .get(&native_id)
            .copied()
            .ok_or_else(|| format!("native node {native_id} is not connected"))?;
        let root_id = root_native_id
            .map(|id| {
                self.nodes
                    .get(&id)
                    .copied()
                    .ok_or_else(|| format!("intersection root {id} is not connected"))
            })
            .transpose()?;
        let target = document
            .get_client_bounding_rect(node_id)
            .ok_or_else(|| format!("Blitz node {node_id} has no client rectangle"))?;
        let root = match root_id {
            Some(id) => document
                .get_client_bounding_rect(id)
                .ok_or_else(|| format!("Blitz root {id} has no client rectangle"))?,
            None => blitz_dom::BoundingRect {
                x: 0.0,
                y: 0.0,
                width: self.viewport_width as f64,
                height: self.viewport_height as f64,
            },
        };
        let expanded_root = blitz_dom::BoundingRect {
            x: root.x - margins[3],
            y: root.y - margins[0],
            width: (root.width + margins[1] + margins[3]).max(0.0),
            height: (root.height + margins[0] + margins[2]).max(0.0),
        };
        let mut intersection = intersect_rect(target, expanded_root);
        let mut root_reached = root_id.is_none() || root_id == Some(node_id);
        let mut ancestor = document.get_node(node_id).and_then(|node| node.parent);
        while let Some(id) = ancestor {
            if Some(id) == root_id {
                root_reached = true;
                break;
            }
            let overflow_x = document
                .computed_property(id, "overflow-x")
                .unwrap_or_else(|| "visible".to_string());
            let overflow_y = document
                .computed_property(id, "overflow-y")
                .unwrap_or_else(|| "visible".to_string());
            if (overflow_x != "visible" || overflow_y != "visible")
                && let Some(clip) = document.get_client_bounding_rect(id)
            {
                if overflow_x != "visible" {
                    intersection = intersect_x(intersection, clip);
                }
                if overflow_y != "visible" {
                    intersection = intersect_y(intersection, clip);
                }
            }
            ancestor = document.get_node(id).and_then(|node| node.parent);
        }
        if !root_reached {
            intersection.width = 0.0;
            intersection.height = 0.0;
        }
        Ok(DomIntersection {
            intersection_rect: DomRect::from_parts(
                intersection.x,
                intersection.y,
                intersection.width,
                intersection.height,
            ),
            root_bounds: DomRect::from_parts(
                expanded_root.x,
                expanded_root.y,
                expanded_root.width,
                expanded_root.height,
            ),
        })
    }

    pub fn hit_test(&self, x: f64, y: f64) -> Result<Option<u64>, String> {
        let document = self
            .document
            .as_ref()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x >= self.viewport_width as f64
            || y >= self.viewport_height as f64
        {
            return Ok(None);
        }
        let Some(hit) = document.hit(x as f32, y as f32) else {
            return Ok(None);
        };
        let mut node_id = Some(hit.node_id);
        while let Some(id) = node_id {
            let node = document.get_node(id);
            if node.is_some_and(|node| node.element_data().is_some())
                && let Some(native_id) = self
                    .nodes
                    .iter()
                    .find_map(|(native, blitz)| (*blitz == id).then_some(*native))
            {
                return Ok(Some(native_id));
            }
            node_id = node.and_then(|node| node.parent);
        }
        Ok(None)
    }

    pub fn rect(&self, native_id: u64) -> Result<DomRect, String> {
        let document = self
            .document
            .as_ref()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;
        let node_id = self
            .nodes
            .get(&native_id)
            .copied()
            .ok_or_else(|| format!("native node {native_id} is not connected"))?;
        let rect = document
            .get_client_bounding_rect(node_id)
            .ok_or_else(|| format!("Blitz node {node_id} has no client rectangle"))?;
        Ok(DomRect::from_parts(rect.x, rect.y, rect.width, rect.height))
    }

    pub fn resize_viewport(&mut self, width: u32, height: u32, scale: f64) {
        let scale = scale.max(f64::EPSILON);
        self.raster_width = width.max(1);
        self.raster_height = height.max(1);
        self.viewport_width = (width as f64 / scale).round().max(1.0) as u32;
        self.viewport_height = (height as f64 / scale).round().max(1.0) as u32;
        self.viewport_scale = scale;
        if let Some(document) = self.document.as_mut() {
            document.set_viewport(Viewport::new(
                self.raster_width,
                self.raster_height,
                scale as f32,
                ColorScheme::Light,
            ));
            document.resolve(self.animation_clock.elapsed().as_secs_f64());
        }
    }

    pub fn render(&mut self, canvases: Vec<CanvasRaster>) -> Result<Vec<u8>, String> {
        self.render_internal(canvases, true)
    }

    pub fn set_canvas_raster(
        &mut self,
        native_id: u64,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<(), String> {
        self.update_canvas_raster(native_id, width, height,
            RasterRegion { x: 0, y: 0, width, height }, &rgba)
    }

    /// Paint a production HUD layer while preserving transparent pixels so it
    /// can be alpha-composited over the native WebGPU surface.
    pub fn suppress_canvas_paint(&mut self, native_id: u64) -> Result<(), String> {
        if let Some(canvas) = self.canvas_pixels.get_mut(&native_id) { canvas.enabled = false; }
        // A detached WebGPU canvas has no HUD raster to suppress.
        let Some(node_id) = self.nodes.get(&native_id).copied() else {
            return Ok(());
        };
        self.document
            .as_mut()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?
            .mutate()
            .clear_canvas_raster(node_id)
            .map_err(str::to_string)
    }

    pub fn paint_overlay(&mut self, scene: &mut impl anyrender::PaintScene) -> Result<(), String> {
        let document = self
            .document
            .as_mut()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;
        paint_scene(
            scene,
            document,
            self.viewport_scale,
            self.raster_width,
            self.raster_height,
            0,
            0,
        );
        Ok(())
    }

    pub fn render_overlay(&mut self) -> Result<Vec<u8>, String> {
        self.render_overlay_region(0, 0, self.raster_width, self.raster_height)
    }

    /// Paint only a physical-pixel HUD region. Blitz clips and translates the
    /// scene at paint time, avoiding a full CPU viewport raster for localized
    /// hover, active, focus, and DOM mutations.
    pub fn render_overlay_region(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, String> {
        self.prepare_canvas_snapshots()?;
        let scale = self.viewport_scale;
        let document = self
            .document
            .as_mut()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;
        Ok(render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| paint_scene_region(scene, document, scale, width, height, x, y),
            width,
            height,
        ))
    }

    fn render_internal(
        &mut self,
        canvases: Vec<CanvasRaster>,
        opaque_page_background: bool,
    ) -> Result<Vec<u8>, String> {
        self.prepare_canvas_snapshots()?;
        let paint_scale = self.viewport_scale;
        let document = self
            .document
            .as_mut()
            .ok_or_else(|| "browser document has not been synchronized".to_string())?;

        // A fullscreen opaque canvas can be copied byte-for-byte only after a
        // probe paint proves that no later DOM paint operation affects any
        // viewport pixel. Unlike the former text-node heuristic, this tests the
        // actual resolved paint tree and catches backgrounds, borders, images,
        // pseudo-elements, and arbitrary overlays.
        let exact_candidate = if canvases.len() == 1 {
            let canvas = &canvases[0];
            let node_id = self.nodes.get(&canvas.native_id).copied();
            let covers_viewport = node_id
                .and_then(|id| document.get_client_bounding_rect(id))
                .is_some_and(|rect| {
                    rect.x.abs() < 0.001
                        && rect.y.abs() < 0.001
                        && (rect.width - self.viewport_width as f64).abs() < 0.001
                        && (rect.height - self.viewport_height as f64).abs() < 0.001
                });
            let opaque = canvas.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255);
            (covers_viewport
                && opaque
                && canvas.width == self.raster_width
                && canvas.height == self.raster_height)
                .then(|| {
                    (
                        node_id.unwrap(),
                        canvas.width,
                        canvas.height,
                        canvas.rgba.clone(),
                    )
                })
        } else {
            None
        };

        for mut canvas in canvases {
            let node_id = self.nodes.get(&canvas.native_id).copied().ok_or_else(|| {
                format!("canvas native node {} is not connected", canvas.native_id)
            })?;
            let expected = canvas.width as usize * canvas.height as usize * 4;
            if canvas.rgba.len() != expected {
                return Err(format!(
                    "canvas {} has {} RGBA bytes, expected {expected}",
                    canvas.native_id,
                    canvas.rgba.len()
                ));
            }
            // GPUCanvasContext readback uses premultiplied-alpha storage while
            // Blitz raster resources accept straight RGBA. Convert exactly once
            // so Blitz's source-over operation does not multiply edge colors a
            // second time; fully transparent pixels remain transparent.
            for pixel in canvas.rgba.chunks_exact_mut(4) {
                let alpha = pixel[3] as u16;
                if alpha != 0 && alpha != 255 {
                    for channel in &mut pixel[..3] {
                        *channel = ((*channel as u16 * 255 + alpha / 2) / alpha).min(255) as u8;
                    }
                }
            }
            document
                .mutate()
                .set_canvas_raster(node_id, canvas.width, canvas.height, Arc::new(canvas.rgba))
                .map_err(|error| {
                    format!("set native canvas {} raster: {error}", canvas.native_id)
                })?;
        }

        // Pixel replacement does not change the already-resolved canvas boxes.

        let width = self.raster_width;
        let height = self.raster_height;
        if let Some((node_id, canvas_width, canvas_height, raw)) = exact_candidate {
            let mut paint_is_canvas_only = true;
            for probe in [[17, 73, 151, 255], [211, 43, 97, 255]] {
                let pixels = probe.repeat(canvas_width as usize * canvas_height as usize);
                document
                    .mutate()
                    .set_canvas_raster(node_id, canvas_width, canvas_height, Arc::new(pixels))
                    .map_err(|error| format!("set canvas proof raster: {error}"))?;
                let proof = render_to_buffer::<VelloCpuImageRenderer, _>(
                    |scene| paint_scene(scene, document, paint_scale, width, height, 0, 0),
                    width,
                    height,
                );
                if !proof.chunks_exact(4).all(|pixel| pixel == probe) {
                    paint_is_canvas_only = false;
                    break;
                }
            }
            document
                .mutate()
                .set_canvas_raster(node_id, canvas_width, canvas_height, Arc::new(raw.clone()))
                .map_err(|error| format!("restore canvas raster: {error}"))?;
            if paint_is_canvas_only {
                return Ok(raw);
            }
        }

        let mut rgba = render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| paint_scene(scene, document, paint_scale, width, height, 0, 0),
            width,
            height,
        );

        // Browser screenshots are opaque. Production HUD overlays retain
        // alpha and are blended over the game frame by the shared GPU queue.
        if opaque_page_background {
            for pixel in rgba.chunks_exact_mut(4) {
                let alpha = pixel[3] as u16;
                if alpha != 255 {
                    let inverse = 255 - alpha;
                    pixel[0] = ((pixel[0] as u16 * alpha + 255 * inverse + 127) / 255) as u8;
                    pixel[1] = ((pixel[1] as u16 * alpha + 255 * inverse + 127) / 255) as u8;
                    pixel[2] = ((pixel[2] as u16 * alpha + 255 * inverse + 127) / 255) as u8;
                    pixel[3] = 255;
                }
            }
        }
        Ok(rgba)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blitz_html::HtmlDocument;

    #[test]
    fn blitz_cpu_paints_html() {
        let mut document = HtmlDocument::from_html(
            "<html><body style='margin:0;background:red;width:100vw;height:100vh'></body></html>",
            DocumentConfig {
                viewport: Some(Viewport::new(32, 32, 1.0, ColorScheme::Light)),
                ..Default::default()
            },
        );
        document.resolve(0.0);
        let pixels = render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| paint_scene(scene, &mut document, 1.0, 32, 32, 0, 0),
            32,
            32,
        );
        assert!(pixels.iter().any(|&value| value != 0));
    }

    #[test]
    fn fixed_popup_hits_above_transformed_canvas() {
        let mut document = HtmlDocument::from_html(
            "<style>html,body{margin:0;width:100%;height:100%;overflow:hidden}main{position:relative;overflow:hidden;width:100%;height:100%}canvas{width:2048px;height:2048px;transform:scale(1.535)}#popup{position:fixed;left:794px;top:63px;width:160px;max-height:148px;overflow-y:auto;z-index:2147483647;padding:4px;border:1px solid;box-sizing:border-box}#option{height:28px;margin-top:28px}</style><main><canvas></canvas></main><div id='popup'><div id='option'></div></div>",
            DocumentConfig { viewport: Some(Viewport::new(1750, 1350, 1.0, ColorScheme::Light)), ..Default::default() },
        );
        document.set_incremental_layout(true);
        document.resolve(0.0);
        let option = document.query_selector("#option").unwrap().unwrap();
        assert_eq!(document.hit(874.0, 110.0).unwrap().node_id, option);
        let popup = document.query_selector("#popup").unwrap().unwrap();
        document.mutate().set_attribute(popup, QualName::new(None, Namespace::from(""), LocalName::from("style")), "transform:translate(300px,200px)");
        document.resolve(0.1);
        assert_eq!(document.hit(1174.0, 310.0).unwrap().node_id, option);
        assert_ne!(document.hit(874.0, 110.0).unwrap().node_id, option);
    }

    #[test]
    fn client_rects_include_nested_transforms_and_keep_layout_offsets() {
        for scale in [1.0, 2.0] {
            let mut document = HtmlDocument::from_html(
                "<style>body{margin:0}#parent{position:fixed;left:0;top:0;width:200px;height:100px;transform:translate(33px,32px);transform-origin:0 0}#child{width:80px;height:20px;transform:scale(2);transform-origin:0 0}</style><div id='parent'><div id='child'></div></div>",
                DocumentConfig { viewport: Some(Viewport::new(800, 500, scale, ColorScheme::Light)), ..Default::default() },
            );
            document.resolve(0.0);
            let parent = document.query_selector("#parent").unwrap().unwrap();
            let child = document.query_selector("#child").unwrap().unwrap();
            let rect = document.get_client_bounding_rect(child).unwrap();
            assert_eq!((rect.x, rect.y, rect.width, rect.height), (33.0, 32.0, 160.0, 40.0));
            let parent_rect = document.get_client_bounding_rect(parent).unwrap();
            assert_eq!((parent_rect.x, parent_rect.y), (33.0, 32.0));
            let offset = document.get_node(child).unwrap().offset_top_left();
            assert_eq!((offset.x, offset.y), (0.0, 0.0));
            assert!(document.hits(40.0, 40.0).iter().any(|hit| hit.node_id == child));
        }
    }

    #[test]
    fn native_text_defaults_edit_select_and_preserve_readonly_values() {
        let mut snapshot: BrowserSnapshot = serde_json::from_value(serde_json::json!({ "nodes": [
            {"id":1,"kind":"element","localName":"html","attributes":[],"children":[2,3]},
            {"id":2,"kind":"element","localName":"style","attributes":[],"children":[],"stylesheetText":
                "body {margin:0} input {width:160px;height:30px;font:16px sans-serif;color:red;background:white}"},
            {"id":3,"kind":"element","localName":"body","attributes":[],"children":[4]},
            {"id":4,"kind":"element","localName":"input","children":[],"attributes":[
                {"localName":"type","value":"text"}, {"localName":"value","value":"hello world"}]}
        ]})).unwrap();
        for node in &mut snapshot.nodes {
            if node.kind == "element" { node.namespace = Some("http://www.w3.org/1999/xhtml".into()); }
        }
        let mut browser = BrowserDocument::new(300, 100);
        browser.sync(1, snapshot.clone(), "file:///tmp/input.html").unwrap();
        browser.set_focus(Some(4)).unwrap();
        let before = browser.render_overlay().unwrap();
        let selected = browser.text_input(4, TextInputAction { action: "key".into(), key: "a".into(), control: true, ..Default::default() }).unwrap();
        assert_eq!((selected.anchor, selected.focus), (0, 11));
        assert_ne!(before, browser.render_overlay().unwrap(), "selection must change the raster");
        let replaced = browser.text_input(4, TextInputAction { action: "key".into(), key: "é".into(), ..Default::default() }).unwrap();
        assert_eq!(replaced.value, "é");
        assert_eq!(replaced.focus, 1);
        assert_ne!(before, browser.render_overlay().unwrap(), "edited glyphs must change the raster");
        snapshot.nodes[3].attributes[1].value = replaced.value;
        browser.sync(2, snapshot.clone(), "file:///tmp/input.html").unwrap();
        assert_eq!(browser.text_input(4, TextInputAction { action: "query".into(), ..Default::default() }).unwrap().focus, 1);
        let deleted = browser.text_input(4, TextInputAction { action: "key".into(), key: "Backspace".into(), ..Default::default() }).unwrap();
        assert_eq!(deleted.value, "");
        let preedit = browser.text_input(4, TextInputAction { action: "imePreedit".into(), key: "に".into(), cursor: Some((3, 3)), ..Default::default() }).unwrap();
        assert_eq!(preedit.value, "に");
        assert!(browser.text_input(4, TextInputAction { action: "imePreedit".into(), key: "に".into(), cursor: Some((1, 1)), ..Default::default() }).is_err());
        let preview = browser.render_overlay().unwrap();
        snapshot.nodes[3].attributes[1].value = preedit.value;
        browser.sync(3, snapshot.clone(), "file:///tmp/input.html").unwrap();
        let committed = browser.text_input(4, TextInputAction { action: "imeCommit".into(), key: "日本".into(), ..Default::default() }).unwrap();
        assert_eq!(committed.value, "日本", "commit must replace preedit, not append it");
        assert_ne!(preview, browser.render_overlay().unwrap());
        browser.text_input(4, TextInputAction { action: "select".into(), anchor: 0, focus: 2, ..Default::default() }).unwrap();
        let pasted = browser.text_input(4, TextInputAction { action: "insert".into(), key: "a😀é".into(), ..Default::default() }).unwrap();
        assert_eq!(pasted.value, "a😀é");
        assert_eq!(pasted.focus, 4);
        snapshot.nodes[3].attributes[1].value = "hello world".into();
        snapshot.nodes[3].attributes.push(BrowserAttributeRecord { local_name: "readonly".into(), namespace: None, prefix: None, value: "".into() });
        browser.sync(4, snapshot, "file:///tmp/input.html").unwrap();
        browser.text_input(4, TextInputAction { action: "select".into(), anchor: 0, focus: 11, ..Default::default() }).unwrap();
        let readonly = browser.text_input(4, TextInputAction { action: "key".into(), key: "x".into(), ..Default::default() }).unwrap();
        assert_eq!(readonly.value, "hello world");
        browser.text_input(4, TextInputAction { action: "down".into(), x: 8.0, y: 15.0, ..Default::default() }).unwrap();
        let dragged = browser.text_input(4, TextInputAction { action: "move".into(), x: 280.0, y: 15.0, ..Default::default() }).unwrap();
        assert_eq!(dragged.focus, 11, "text drag must extend beyond the input border");
        assert!(dragged.anchor < dragged.focus);
    }

    #[test]
    fn text_selection_uses_inverse_transforms_outside_the_input() {
        for scale in [1.0, 2.0] {
            let mut snapshot: BrowserSnapshot = serde_json::from_value(serde_json::json!({ "nodes": [
                {"id":1,"kind":"element","localName":"html","attributes":[],"children":[2,3]},
                {"id":2,"kind":"element","localName":"style","attributes":[],"children":[],"stylesheetText":
                    "body{margin:0} input{position:absolute;left:100px;top:100px;width:160px;height:30px;font:16px sans-serif;transform-origin:0 0;transform:rotate(30deg) scale(1.5)}"},
                {"id":3,"kind":"element","localName":"body","attributes":[],"children":[4]},
                {"id":4,"kind":"element","localName":"input","children":[],"attributes":[{"localName":"value","value":"hello world"}]}
            ]})).unwrap();
            for node in &mut snapshot.nodes { node.namespace = Some("http://www.w3.org/1999/xhtml".into()); }
            let mut browser = BrowserDocument::new(800, 600);
            browser.resize_viewport(800, 600, scale);
            browser.sync(1, snapshot, "file:///tmp/transformed-input.html").unwrap();
            browser.set_focus(Some(4)).unwrap();
            let point = |x: f32| (100.0 + 1.5 * (x * 30f32.to_radians().cos() - 15.0 * 0.5), 100.0 + 1.5 * (x * 0.5 + 15.0 * 30f32.to_radians().cos()));
            let (x, y) = point(5.0);
            browser.text_input(4, TextInputAction { action: "down".into(), x, y, ..Default::default() }).unwrap();
            let (x, y) = point(250.0);
            let selected = browser.text_input(4, TextInputAction { action: "move".into(), x, y, ..Default::default() }).unwrap();
            assert_eq!(selected.focus, 11, "transformed drag at scale {scale}");
            assert!(selected.anchor < selected.focus);
        }
    }

    #[test]
    fn number_inputs_paint_values_after_details_open() {
        for initially_open in [true, false] {
            let mut document = HtmlDocument::from_html(
                &format!("<style>body{{margin:0;font:13px sans-serif}}.grid{{display:grid;grid-template-columns:1fr 1fr;gap:5px}}label{{font-size:10px}}input{{width:100%;margin-top:2px;padding:4px;border:1px solid black;color:red;background:white}}</style><details {}><summary>Document</summary><div class='grid'><label>Width<input id='width' type='number' value='2048'></label><label>Height<input id='height' type='number' value='2048'></label></div></details>", if initially_open { "open" } else { "" }),
                DocumentConfig {
                    viewport: Some(Viewport::new(300, 160, 1.0, ColorScheme::Light)),
                    font_ctx: Some(build_browser_font_ctx(
                        include_bytes!("../vendor/fonts/LiberationSans-Regular.ttf"),
                        include_bytes!("../vendor/fonts/JetBrainsMonoNerdFontMono-Regular.ttf"),
                    )),
                    ..Default::default()
                },
            );
            document.set_incremental_layout(true);
            document.resolve(0.0);
            let details = document.query_selector("details").unwrap().unwrap();
            document.mutate().set_attribute(details, QualName::new(None, Namespace::from(""), LocalName::from("open")), "");
            document.resolve(0.1);
            let pixels = render_to_buffer::<VelloCpuImageRenderer, _>(
                |scene| paint_scene(scene, &mut document, 1.0, 300, 160, 0, 0),
                300, 160,
            );
            for selector in ["#width", "#height"] {
                let id = document.query_selector(selector).unwrap().unwrap();
                let rect = document.get_client_bounding_rect(id).unwrap();
                let mut ink = 0;
                for y in rect.y.max(0.0) as usize..((rect.y + rect.height) as usize).min(160) {
                    for x in rect.x.max(0.0) as usize..((rect.x + rect.width) as usize).min(300) {
                        let pixel = &pixels[(y * 300 + x) * 4..][..4];
                        if pixel[0] > 100 && pixel[1] < 80 && pixel[2] < 80 { ink += 1; }
                    }
                }
                assert!(ink > 5, "{selector}: missing number glyphs, initially_open={initially_open}, rect={rect:?}");
            }
        }
    }

    #[test]
    fn native_select_displays_only_its_current_label() {
        let mut document = HtmlDocument::from_html(
            "<style>body{margin:0}select{display:inline-block;width:160px;height:30px}</style><select><option id='first' selected>Off</option><option id='second'>Smoothing</option></select>",
            DocumentConfig {
                viewport: Some(Viewport::new(200, 80, 1.0, ColorScheme::Light)),
                font_ctx: Some(build_browser_font_ctx(
                    include_bytes!("../vendor/fonts/LiberationSans-Regular.ttf"),
                    include_bytes!("../vendor/fonts/JetBrainsMonoNerdFontMono-Regular.ttf"),
                )),
                ..Default::default()
            },
        );
        document.resolve(0.0);
        let first = document.query_selector("#first").unwrap().unwrap();
        let second = document.query_selector("#second").unwrap().unwrap();
        assert!(document.get_client_bounding_rect(first).unwrap().width > 0.0,
            "first display={:?}, second display={:?}, first layout={:?}",
            document.computed_property(first, "display"),
            document.computed_property(second, "display"),
            document.get_node(first).unwrap().final_layout());
        assert_eq!(document.get_client_bounding_rect(second).unwrap().width, 0.0);
        document.mutate().clear_attribute(first, QualName::new(None, Namespace::from(""), LocalName::from("selected")));
        document.mutate().set_attribute(second, QualName::new(None, Namespace::from(""), LocalName::from("selected")), "");
        document.resolve(0.1);
        assert_eq!(document.get_client_bounding_rect(first).unwrap().width, 0.0);
        assert!(document.get_client_bounding_rect(second).unwrap().width > 0.0);
    }

    #[test]
    fn hover_transitions_advance_and_return_to_idle() {
        let mut snapshot: BrowserSnapshot = serde_json::from_value(serde_json::json!({ "nodes": [
            {"id":1,"kind":"element","localName":"html","attributes":[],"children":[2,3]},
            {"id":2,"kind":"element","localName":"style","attributes":[],"children":[],"stylesheetText":
                "body {margin:0} button {position:absolute;left:40px;top:40px;width:80px;height:40px;border:0;background:rgb(255,0,0);transition:background-color 1s linear} button:hover {background:rgb(0,0,255)}"},
            {"id":3,"kind":"element","localName":"body","attributes":[],"children":[4]},
            {"id":4,"kind":"element","localName":"button","attributes":[],"children":[]}
        ]})).unwrap();
        for node in &mut snapshot.nodes {
            node.namespace = Some("http://www.w3.org/1999/xhtml".to_string());
        }
        let mut browser = BrowserDocument::new(200, 120);
        browser.sync(1, snapshot, "file:///tmp/hover.html").unwrap();
        let initial = browser.render_overlay().unwrap();
        assert!(!browser.is_animating());
        browser.set_pointer_state(0, 70.0, 60.0).unwrap();
        assert!(browser.is_animating());
        browser.animation_clock -= std::time::Duration::from_millis(500);
        browser.advance_animations();
        let middle = browser.render_overlay().unwrap();
        assert_ne!(middle, initial);
        assert!(browser.is_animating());
        browser.animation_clock -= std::time::Duration::from_secs(2);
        browser.advance_animations();
        let final_pixels = browser.render_overlay().unwrap();
        assert_ne!(final_pixels, middle);
        assert!(!browser.is_animating());
        assert_eq!(&final_pixels[(60 * 200 + 70) * 4..][..4], &[0, 0, 255, 255]);
    }

    #[test]
    fn incremental_layout_matches_full_layout_after_ui_changes() {
        let mut snapshot: BrowserSnapshot = serde_json::from_value(serde_json::json!({ "nodes": [
            {"id":1,"kind":"element","localName":"html","attributes":[],"children":[2,3]},
            {"id":2,"kind":"element","localName":"style","attributes":[],"children":[],"stylesheetText":
                "body {margin:0} button {width:80px;height:30px} button:hover {width:100px;background:red}"},
            {"id":3,"kind":"element","localName":"body","attributes":[],"children":[4,6]},
            {"id":4,"kind":"element","localName":"button","attributes":[],"children":[5]},
            {"id":5,"kind":"text","text":"Brush","attributes":[],"children":[]},
            {"id":6,"kind":"element","localName":"input","children":[],"attributes":[
                {"localName":"type","value":"range"}, {"localName":"value","value":"10"}]}
        ]})).unwrap();
        for node in &mut snapshot.nodes {
            if node.kind == "element" {
                node.namespace = Some("http://www.w3.org/1999/xhtml".to_string());
            }
        }
        let mut incremental = BrowserDocument::new(240, 100);
        let mut full = BrowserDocument::new(240, 100);
        for browser in [&mut incremental, &mut full] {
            browser
                .sync(1, snapshot.clone(), "file:///tmp/layout.html")
                .unwrap();
            assert!(browser.document.as_ref().unwrap().incremental_layout());
        }
        full.document
            .as_mut()
            .unwrap()
            .set_incremental_layout(false);
        for step in 0..7 {
            match step {
                1 => snapshot.nodes[4].text = Some("New label".to_string()),
                2 => snapshot.nodes[5].attributes[1].value = "90".to_string(),
                3 => {
                    snapshot.nodes[1].stylesheet_text = Some(
                        "body {margin:0} button {width:120px;height:40px;background:blue}"
                            .to_string(),
                    )
                }
                4 => snapshot.nodes[3].attributes.push(BrowserAttributeRecord {
                    local_name: "style".to_string(),
                    namespace: None,
                    prefix: None,
                    value: "display:none".to_string(),
                }),
                5 => snapshot.nodes[3].attributes.clear(),
                6 => {
                    snapshot.nodes[2].children = vec![6];
                    snapshot.nodes.retain(|node| node.id != 4 && node.id != 5);
                }
                _ => {}
            }
            for browser in [&mut incremental, &mut full] {
                browser
                    .sync(step + 2, snapshot.clone(), "file:///tmp/layout.html")
                    .unwrap();
                browser.set_pointer_state(0, 10.0, 10.0).unwrap();
                if step == 5 {
                    browser.resize_viewport(260, 100, 1.0);
                }
            }
            assert_eq!(
                incremental.render_overlay().unwrap(),
                full.render_overlay().unwrap(),
                "incremental paint differs at step {step}"
            );
            let actual = incremental.rect(6).unwrap();
            let expected = full.rect(6).unwrap();
            assert_eq!(
                (actual.x, actual.y, actual.width, actual.height),
                (expected.x, expected.y, expected.width, expected.height)
            );
        }
    }

    #[test]
    fn structured_snapshot_processes_embedded_styles() {
        let html_ns = Some("http://www.w3.org/1999/xhtml".to_string());
        let element = |id, name: &str, children| BrowserNodeRecord {
            id,
            kind: "element".to_string(),
            local_name: Some(name.to_string()),
            namespace: html_ns.clone(),
            prefix: None,
            attributes: Vec::new(),
            text: None,
            stylesheet_text: None,
            checked: None,
            children,
        };
        let text = |id, value: &str| BrowserNodeRecord {
            id,
            kind: "text".to_string(),
            local_name: None,
            namespace: None,
            prefix: None,
            attributes: Vec::new(),
            text: Some(value.to_string()),
            stylesheet_text: None,
            checked: None,
            children: Vec::new(),
        };
        let mut snapshot = BrowserSnapshot {
            nodes: vec![
                element(1, "html", vec![2, 5]),
                element(2, "head", vec![3]),
                element(3, "style", vec![4]),
                text(4, "#target { display:flex }"),
                element(5, "body", vec![6]),
                BrowserNodeRecord {
                    attributes: vec![BrowserAttributeRecord {
                        local_name: "id".to_string(),
                        namespace: None,
                        prefix: None,
                        value: "target".to_string(),
                    }],
                    ..element(6, "div", Vec::new())
                },
            ],
        };
        let mut browser = BrowserDocument::new(800, 500);
        browser
            .sync(1, snapshot.clone(), "file:///tmp/test.html")
            .unwrap();
        let blitz_id = browser.nodes[&6];
        assert_eq!(browser.computed_property(6, "display", "").unwrap(), "flex");

        snapshot.nodes[3].text = Some("#target { display:grid }".to_string());
        browser
            .sync(2, snapshot.clone(), "file:///tmp/test.html")
            .unwrap();
        assert_eq!(
            browser.nodes[&6], blitz_id,
            "reconciliation must preserve node identity"
        );
        assert_eq!(browser.computed_property(6, "display", "").unwrap(), "grid");

        snapshot.nodes.retain(|record| record.id != 6);
        snapshot
            .nodes
            .iter_mut()
            .find(|record| record.id == 5)
            .unwrap()
            .children
            .clear();
        browser.sync(3, snapshot, "file:///tmp/test.html").unwrap();
        assert!(
            !browser.nodes.contains_key(&6),
            "detached nodes must be destroyed"
        );
    }

    #[test]
    fn reactive_text_reconciliation_preserves_unchanged_paint_content() {
        let html_ns = Some("http://www.w3.org/1999/xhtml".to_string());
        let element = |id, name: &str, attributes, children| BrowserNodeRecord {
            id,
            kind: "element".to_string(),
            local_name: Some(name.to_string()),
            namespace: html_ns.clone(),
            prefix: None,
            attributes,
            text: None,
            stylesheet_text: None,
            checked: None,
            children,
        };
        let text = |id, value: &str| BrowserNodeRecord {
            id,
            kind: "text".to_string(),
            local_name: None,
            namespace: None,
            prefix: None,
            attributes: Vec::new(),
            text: Some(value.to_string()),
            stylesheet_text: None,
            checked: None,
            children: Vec::new(),
        };
        let id = |value: &str| {
            vec![BrowserAttributeRecord {
                local_name: "id".to_string(),
                namespace: None,
                prefix: None,
                value: value.to_string(),
            }]
        };
        let mut snapshot = BrowserSnapshot {
            nodes: vec![
                element(1, "html", Vec::new(), vec![2, 5]),
                element(2, "head", Vec::new(), vec![3]),
                BrowserNodeRecord {
                    stylesheet_text: Some("#app{position:fixed;left:24px;top:24px;z-index:10;width:250px;padding:18px;background:rgb(3,12,25);border:1px solid #57baff;color:white;font:16px sans-serif}button{padding:9px 14px;cursor:pointer}li{color:#aee4ff}canvas{position:fixed;inset:0;z-index:0}".to_string()),
                    ..element(3, "style", Vec::new(), vec![4])
                },
                text(4, ""),
                element(5, "body", Vec::new(), vec![6, 22]),
                element(6, "div", id("app"), vec![7, 9, 11, 13]),
                element(7, "h2", Vec::new(), vec![8]),
                text(8, "Vue 3 native smoke"),
                element(9, "button", Vec::new(), vec![10]),
                text(10, "count 0"),
                element(11, "p", Vec::new(), vec![12]),
                text(12, "computed 0"),
                element(13, "ul", Vec::new(), vec![14, 16, 18]),
                element(14, "li", Vec::new(), vec![15]),
                text(15, "reactivity"),
                element(16, "li", Vec::new(), vec![17]),
                text(17, "templates"),
                element(18, "li", Vec::new(), vec![19]),
                text(19, "events"),
                element(
                    22,
                    "canvas",
                    vec![
                        BrowserAttributeRecord {
                            local_name: "width".to_string(),
                            namespace: None,
                            prefix: None,
                            value: "400".to_string(),
                        },
                        BrowserAttributeRecord {
                            local_name: "height".to_string(),
                            namespace: None,
                            prefix: None,
                            value: "300".to_string(),
                        },
                    ],
                    Vec::new(),
                ),
            ],
        };
        let mut browser = BrowserDocument::new(400, 300);
        browser
            .sync(1, snapshot.clone(), "file:///tmp/vue.html")
            .unwrap();
        assert_eq!(
            browser.computed_property(9, "cursor", "").unwrap(),
            "pointer"
        );
        browser.suppress_canvas_paint(22).unwrap();
        let before = browser.render_overlay().unwrap();
        assert!(browser.set_focus(Some(9)).unwrap());
        snapshot.nodes.retain(|node| node.id != 10 && node.id != 12);
        snapshot
            .nodes
            .iter_mut()
            .find(|node| node.id == 9)
            .unwrap()
            .children = vec![20];
        snapshot
            .nodes
            .iter_mut()
            .find(|node| node.id == 11)
            .unwrap()
            .children = vec![21];
        snapshot.nodes.push(text(20, "count 1"));
        snapshot.nodes.push(text(21, "computed 2"));
        browser.sync(2, snapshot, "file:///tmp/vue.html").unwrap();
        browser.suppress_canvas_paint(22).unwrap();
        assert!(
            browser
                .document
                .as_ref()
                .unwrap()
                .get_node(browser.nodes[&22])
                .unwrap()
                .element_data()
                .unwrap()
                .raster_image_data()
                .is_none(),
            "the separately presented GPU canvas must not enter the HUD scene"
        );
        let after = browser.render_overlay().unwrap();

        let unchanged_region_alpha = |pixels: &[u8]| {
            (24..274)
                .flat_map(|x| (24..95).map(move |y| pixels[(y * 400 + x) * 4 + 3] as u64))
                .sum::<u64>()
        };
        assert!(unchanged_region_alpha(&before) > 100_000);
        assert_eq!(
            unchanged_region_alpha(&after),
            unchanged_region_alpha(&before),
            "updating reactive text must not drop unchanged panel/title paint"
        );
    }

    #[test]
    fn blitz_exposes_stylo_computed_properties() {
        let mut document = HtmlDocument::from_html(
            "<style>#target { display: flex; z-index: 7; color: rgb(1, 2, 3); --accent: #123456 }</style><div id='target'></div>",
            DocumentConfig::default(),
        );
        document.resolve(0.0);
        let id = document.get_element_by_id("target").unwrap();
        assert_eq!(
            document.computed_property(id, "display").as_deref(),
            Some("flex")
        );
        assert_eq!(
            document.computed_property(id, "z-index").as_deref(),
            Some("7")
        );
        assert_eq!(
            document.computed_property(id, "color").as_deref(),
            Some("rgb(1, 2, 3)")
        );
        assert_eq!(
            document.computed_property(id, "--accent").as_deref(),
            Some("#123456")
        );
    }

    #[test]
    fn text_hits_stay_inside_the_text_line() {
        let mut document = HtmlDocument::from_html(
            "<style>body{margin:0}button{display:block;width:100px;height:40px;margin-bottom:50px}</style><button id='first'>Brush</button><button id='second'>Clear</button>",
            DocumentConfig {
                viewport: Some(Viewport::new(200, 200, 1.0, ColorScheme::Light)),
                ..Default::default()
            },
        );
        document.resolve(0.0);
        let first = document.get_element_by_id("first").unwrap();
        let second = document.get_element_by_id("second").unwrap();
        for (y, expected) in [(20.0, first), (110.0, second)] {
            let mut id = document.hit(40.0, y).unwrap().node_id;
            while document.get_node(id).unwrap().element_data().is_none() {
                id = document.get_node(id).unwrap().parent.unwrap();
            }
            assert_eq!(id, expected);
        }
        assert_ne!(document.hit(40.0, 70.0).unwrap().node_id, second);
    }

    #[test]
    fn grid_overflow_accepts_wheel_scroll() {
        let html = format!(
            "<style>#grid{{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:8px;max-height:238px;width:289px;overflow-y:auto}}button{{height:70px}}</style><div id='grid'>{}</div>",
            "<button>Brush</button>".repeat(80)
        );
        let mut document = HtmlDocument::from_html(
            &html,
            DocumentConfig {
                viewport: Some(Viewport::new(400, 500, 1.0, ColorScheme::Light)),
                ..Default::default()
            },
        );
        document.resolve(0.0);
        let id = document.get_element_by_id("grid").unwrap();
        document.mutate().set_attribute(
            id,
            QualName::new(None, Namespace::from(""), LocalName::from("style")),
            "gap:9px",
        );
        document.resolve(0.0);
        assert!(
            document.get_node(id).unwrap().scrollable_overflow().y1 > 1500.0,
            "cached child bounds must retain their parent-relative positions"
        );
        assert!(
            document.scroll_node_by_has_changed(id, 0.0, -200.0, |_| {}),
            "grid layout: {:?}, overflow: {:?}",
            document.get_node(id).unwrap().final_layout(),
            document.get_node(id).unwrap().scrollable_overflow()
        );
        assert_eq!(document.get_node(id).unwrap().scroll_offset().y, 200.0);
    }

    #[test]
    fn scroll_does_not_change_layout_offsets() {
        let mut snapshot: BrowserSnapshot = serde_json::from_value(serde_json::json!({ "nodes": [
            {"id":1,"kind":"element","localName":"html","attributes":[],"children":[2,3]},
            {"id":2,"kind":"element","localName":"style","attributes":[],"children":[],"stylesheetText":
                "body{margin:0}#popup{position:fixed;left:40px;top:80px;width:160px;height:100px;padding:4px;border:1px solid;overflow:auto}#row{display:block;width:100px;height:28px;margin-top:168px;margin-bottom:100px}"},
            {"id":3,"kind":"element","localName":"body","attributes":[],"children":[4]},
            {"id":4,"kind":"element","localName":"div","attributes":[{"localName":"id","value":"popup"}],"children":[5]},
            {"id":5,"kind":"element","localName":"button","attributes":[{"localName":"id","value":"row"}],"children":[]}
        ]})).unwrap();
        for node in &mut snapshot.nodes {
            node.namespace = Some("http://www.w3.org/1999/xhtml".to_string());
        }
        let mut browser = BrowserDocument::new(400, 400);
        browser.sync(1, snapshot, "file:///tmp/scroll.html").unwrap();
        let before = browser.box_metrics(5).unwrap();
        assert_eq!(before.offset_parent, Some(4));
        assert_eq!(before.offset_top, 172);
        assert!(browser.set_scroll(4, 0.0, 168.0).unwrap());
        let after = browser.box_metrics(5).unwrap();
        assert_eq!((after.offset_left, after.offset_top), (before.offset_left, before.offset_top));
    }

    #[test]
    fn range_has_size_and_a_movable_painted_thumb() {
        let mut document = HtmlDocument::from_html(
            "<style>body{margin:0}input{color:red}</style><input id='range' type='range' min='0' max='100' value='0'>",
            DocumentConfig {
                viewport: Some(Viewport::new(160, 40, 1.0, ColorScheme::Light)),
                ..Default::default()
            },
        );
        document.resolve(0.0);
        let id = document.get_element_by_id("range").unwrap();
        let rect = document.get_client_bounding_rect(id).unwrap();
        assert!(rect.width >= 100.0 && rect.height >= 12.0);
        let before = render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| paint_scene(scene, &mut document, 1.0, 160, 40, 0, 0),
            160,
            40,
        );
        document.mutate().set_attribute(
            id,
            QualName::new(None, Namespace::from(""), LocalName::from("value")),
            "100",
        );
        document.resolve(0.0);
        let after = render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| paint_scene(scene, &mut document, 1.0, 160, 40, 0, 0),
            160,
            40,
        );
        assert_ne!(before, after, "the thumb must move when the value changes");
    }

    #[test]
    fn hit_tests_exclude_clipped_and_hidden_controls() {
        let mut document = HtmlDocument::from_html(
            "<style>body{margin:0}#clip{width:100px;height:40px;overflow:auto}#child{height:200px}#behind{width:100px;height:40px}#hidden{display:none}</style><div id='clip'><button id='child'>long</button></div><button id='behind'>behind</button><button id='hidden'>hidden</button>",
            DocumentConfig {
                viewport: Some(Viewport::new(200, 200, 1.0, ColorScheme::Light)),
                ..Default::default()
            },
        );
        document.resolve(0.0);
        let clip = document.get_element_by_id("clip").unwrap();
        let child = document.get_element_by_id("child").unwrap();
        let hidden = document.get_element_by_id("hidden").unwrap();
        for (x, y) in [(10.0, 60.0), (10.0, 100.0)] {
            let hit = document.hit(x, y).map(|hit| hit.node_id);
            assert_ne!(hit, Some(child), "a clipped child cannot receive a hit");
            assert_ne!(hit, Some(hidden), "a hidden button cannot receive a hit");
        }
        assert!(document.scroll_node_by_has_changed(clip, 0.0, -80.0, |_| {}));
        assert_eq!(document.get_node(clip).unwrap().scroll_offset().y, 80.0);
        assert_ne!(document.hit(10.0, 60.0).map(|hit| hit.node_id), Some(child));
    }

    #[test]
    fn detached_surface_canvas_has_no_hud_raster() {
        let mut browser = BrowserDocument::new(32, 32);
        assert!(browser.suppress_canvas_paint(1).is_ok());
        assert!(browser.set_canvas_raster(1, 1, 1, vec![0; 4]).is_err());
    }

    #[test]
    fn blitz_cpu_paints_canvas_raster() {
        let mut document = HtmlDocument::from_html(
            "<html><body style='margin:0'><canvas id='c' style='display:block;width:32px;height:32px' width='32' height='32'></canvas></body></html>",
            DocumentConfig {
                viewport: Some(Viewport::new(32, 32, 1.0, ColorScheme::Light)),
                ..Default::default()
            },
        );
        document.resolve(0.0);
        let id = document.get_element_by_id("c").unwrap();
        document
            .mutate()
            .set_canvas_raster(id, 32, 32, Arc::new(vec![255; 32 * 32 * 4]))
            .unwrap();
        document.resolve(0.0);
        let pixels = render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| paint_scene(scene, &mut document, 1.0, 32, 32, 0, 0),
            32,
            32,
        );
        assert!(pixels.iter().any(|&value| value != 0));
    }
}
