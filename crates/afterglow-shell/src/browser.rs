use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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
    nodes: HashMap<u64, usize>,
    viewport_width: u32,
    viewport_height: u32,
    raster_width: u32,
    raster_height: u32,
    viewport_scale: f64,
    epoch: u64,
}

impl BrowserDocument {
    pub fn new(viewport_width: u32, viewport_height: u32) -> Self {
        Self {
            document: None,
            nodes: HashMap::new(),
            viewport_width,
            viewport_height,
            raster_width: viewport_width,
            raster_height: viewport_height,
            viewport_scale: 1.0,
            epoch: 0,
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
                    "comment" => mutator.create_comment_node(),
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
        document.resolve(0.0);
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
                    "comment" => mutator.create_comment_node(),
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
        for record in &snapshot.nodes {
            if let Some(css) = &record.stylesheet_text {
                document.set_stylesheet_text_for_node(self.nodes[&record.id], css);
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
        document.resolve(0.0);
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
            "::before" => document.get_node(element_id).and_then(|node| node.before),
            "::after" => document.get_node(element_id).and_then(|node| node.after),
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
            document.resolve(0.0);
        }
        Ok(changed)
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
            document.resolve(0.0);
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
            .scroll_offset;
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
        let layout = node.final_layout;
        let client_width = (layout.size.width - layout.border.left - layout.border.right).max(0.0);
        let client_height =
            (layout.size.height - layout.border.top - layout.border.bottom).max(0.0);
        let scroll_width = client_width.max(node.scrollable_overflow.x1 as f32);
        let scroll_height = client_height.max(node.scrollable_overflow.y1 as f32);

        let mut parent_id = node.parent;
        let mut offset_parent_id = None;
        while let Some(id) = parent_id {
            let parent = document
                .get_node(id)
                .ok_or_else(|| format!("missing Blitz ancestor {id}"))?;
            let is_body = parent
                .element_data()
                .is_some_and(|element| element.name.local.as_ref() == "body");
            let is_positioned = document.node_is_positioned(id);
            if is_body || is_positioned {
                offset_parent_id = Some(id);
                break;
            }
            parent_id = parent.parent;
        }
        let rect = document
            .get_client_bounding_rect(node_id)
            .ok_or_else(|| format!("Blitz node {node_id} has no client rectangle"))?;
        let (offset_left, offset_top) = offset_parent_id
            .and_then(|id| document.get_client_bounding_rect(id))
            .map_or((rect.x, rect.y), |parent| {
                (rect.x - parent.x, rect.y - parent.y)
            });
        let offset_parent = offset_parent_id.and_then(|id| {
            self.nodes
                .iter()
                .find_map(|(native, blitz)| (*blitz == id).then_some(*native))
        });

        Ok(DomBoxMetrics {
            client_width: client_width.round() as i32,
            client_height: client_height.round() as i32,
            client_left: layout.border.left.round() as i32,
            client_top: layout.border.top.round() as i32,
            offset_width: layout.size.width.round() as i32,
            offset_height: layout.size.height.round() as i32,
            offset_left: offset_left.round() as i32,
            offset_top: offset_top.round() as i32,
            offset_parent,
            scroll_width: scroll_width.round() as i32,
            scroll_height: scroll_height.round() as i32,
            scroll_left: node.scroll_offset.x,
            scroll_top: node.scroll_offset.y,
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
        let reverse_nodes: HashMap<usize, u64> = self
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
            document.resolve(0.0);
        }
    }

    pub fn render(&mut self, canvases: Vec<CanvasRaster>) -> Result<Vec<u8>, String> {
        self.render_internal(canvases, true)
    }

    /// Paint a production HUD layer while preserving transparent pixels so it
    /// can be alpha-composited over the native WebGPU surface.
    pub fn suppress_canvas_paint(&mut self, native_id: u64) -> Result<(), String> {
        let node_id = self
            .nodes
            .get(&native_id)
            .copied()
            .ok_or_else(|| format!("native canvas {native_id} is not connected"))?;
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
