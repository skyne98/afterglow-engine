use std::sync::Arc;

use blitz_dom::DocumentConfig;
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_traits::shell::{ColorScheme, Viewport};

fn layout_doc(html: &str) -> HtmlDocument {
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Light)),
            html_parser_provider: Some(Arc::new(HtmlProvider) as _),
            ..Default::default()
        },
    );
    doc.resolve(0.0);
    doc
}

#[test]
fn default_inline_canvas_has_default_intrinsic_size() {
    let doc = layout_doc("<html><body><canvas id='c'></canvas></body></html>");
    let id = doc.query_selector("#c").unwrap().unwrap();
    let layout = doc.get_node(id).unwrap().final_layout;
    assert_eq!((layout.size.width, layout.size.height), (300.0, 150.0));
}

#[test]
fn default_inline_canvas_uses_dimension_attributes() {
    let doc =
        layout_doc("<html><body><canvas id='c' width='200' height='120'></canvas></body></html>");
    let id = doc.query_selector("#c").unwrap().unwrap();
    let layout = doc.get_node(id).unwrap().final_layout;
    assert_eq!((layout.size.width, layout.size.height), (200.0, 120.0));
}

#[test]
fn css_size_overrides_canvas_intrinsic_size_without_changing_display() {
    let doc = layout_doc(
        "<html><body><canvas id='c' width='200' height='100' style='width:80px;height:40px'></canvas></body></html>",
    );
    let id = doc.query_selector("#c").unwrap().unwrap();
    let layout = doc.get_node(id).unwrap().final_layout;
    assert_eq!((layout.size.width, layout.size.height), (80.0, 40.0));
}
