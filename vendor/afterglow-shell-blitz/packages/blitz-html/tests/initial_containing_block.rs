use std::sync::Arc;

use blitz_dom::DocumentConfig;
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_traits::shell::{ColorScheme, Viewport};

fn layout_doc(html: &str) -> HtmlDocument {
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 500, 1.0, ColorScheme::Light)),
            html_parser_provider: Some(Arc::new(HtmlProvider) as _),
            ..Default::default()
        },
    );
    doc.resolve(0.0);
    doc
}

#[test]
fn absolute_percent_size_uses_initial_containing_block_without_positioned_ancestor() {
    let doc = layout_doc(
        "<html><body><canvas id='c' style='position:absolute;left:0;top:0;width:100%;height:100%'></canvas></body></html>",
    );
    let id = doc.query_selector("#c").unwrap().unwrap();
    let rect = doc.get_client_bounding_rect(id).unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (0.0, 0.0, 800.0, 500.0)
    );
}

#[test]
fn absolute_percent_size_uses_positioned_ancestor_when_present() {
    let doc = layout_doc(
        "<html><body><div style='position:relative;width:320px;height:180px'><canvas id='c' style='position:absolute;left:0;top:0;width:100%;height:100%'></canvas></div></body></html>",
    );
    let id = doc.query_selector("#c").unwrap().unwrap();
    let rect = doc.get_client_bounding_rect(id).unwrap();
    assert_eq!((rect.width, rect.height), (320.0, 180.0));
}

#[test]
fn fixed_bottom_right_uses_viewport_even_inside_positioned_ancestor() {
    let doc = layout_doc(
        "<html><body><div style='position:relative;width:320px;height:180px'><div id='panel' style='position:fixed;right:10px;bottom:20px;width:200px;height:100px'></div></div></body></html>",
    );
    let id = doc.query_selector("#panel").unwrap().unwrap();
    let rect = doc.get_client_bounding_rect(id).unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (590.0, 380.0, 200.0, 100.0)
    );
}

#[test]
fn fixed_position_and_initial_containing_block_use_logical_hidpi_size() {
    let mut doc = HtmlDocument::from_html(
        "<html><body><div id='panel' style='position:fixed;right:10px;bottom:20px;width:200px;height:100px'></div></body></html>",
        DocumentConfig {
            viewport: Some(Viewport::new(1600, 1000, 2.0, ColorScheme::Light)),
            html_parser_provider: Some(Arc::new(HtmlProvider) as _),
            ..Default::default()
        },
    );
    doc.resolve(0.0);
    let id = doc.query_selector("#panel").unwrap().unwrap();
    let rect = doc.get_client_bounding_rect(id).unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (590.0, 380.0, 200.0, 100.0)
    );
}
