use std::sync::Arc;

use blitz_dom::DocumentConfig;
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_traits::shell::{ColorScheme, Viewport};

fn resolved_doc(html: &str) -> HtmlDocument {
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
fn match_media_uses_the_document_stylo_device() {
    let doc = resolved_doc("<html><body></body></html>");
    assert!(doc.media_query_matches("screen and (min-width: 800px)"));
    assert!(!doc.media_query_matches("print"));
    assert!(doc.media_query_matches("(orientation: landscape)"));
    assert!(!doc.media_query_matches("(orientation: portrait)"));
    assert!(!doc.media_query_matches("(prefers-color-scheme: dark)"));
    assert!(!doc.media_query_matches("(dynamic-range: high)"));
    assert!(doc.media_query_matches("print, (width: 800px)"));
}

#[test]
fn all_hits_are_returned_in_front_to_back_paint_order() {
    let doc = resolved_doc(
        "<html><body style='margin:0'><div id='back' style='position:absolute;inset:0;width:100px;height:100px'></div><div id='front' style='position:absolute;inset:0;width:100px;height:100px'></div></body></html>",
    );
    let front = doc.query_selector("#front").unwrap().unwrap();
    let back = doc.query_selector("#back").unwrap().unwrap();
    let hits: Vec<_> = doc
        .hits(10.0, 10.0)
        .into_iter()
        .map(|hit| hit.node_id)
        .collect();
    let front_index = hits.iter().position(|id| *id == front).unwrap();
    let back_index = hits.iter().position(|id| *id == back).unwrap();
    assert!(front_index < back_index, "hits were {hits:?}");
}

#[test]
fn overflow_visible_descendants_are_hit_outside_parent_box() {
    let doc = resolved_doc(
        "<html><body style='margin:0'><div id='parent' style='position:fixed;left:10px;top:10px;width:50px;height:50px'><a id='child' href='#' style='position:absolute;left:60px;top:0;width:100px;height:30px'>link</a></div></body></html>",
    );
    let child = doc.query_selector("#child").unwrap().unwrap();
    let hits: Vec<_> = doc
        .hits(80.0, 20.0)
        .into_iter()
        .map(|hit| hit.node_id)
        .collect();
    assert!(hits.contains(&child), "hits were {hits:?}");
}

#[test]
fn all_hits_respect_pointer_events_none() {
    let doc = resolved_doc(
        "<html><body style='margin:0'><div id='back' style='position:absolute;inset:0;width:100px;height:100px'></div><div id='front' style='pointer-events:none;position:absolute;inset:0;width:100px;height:100px'></div></body></html>",
    );
    let front = doc.query_selector("#front").unwrap().unwrap();
    let back = doc.query_selector("#back").unwrap().unwrap();
    let hits: Vec<_> = doc
        .hits(10.0, 10.0)
        .into_iter()
        .map(|hit| hit.node_id)
        .collect();
    assert!(!hits.contains(&front), "hits were {hits:?}");
    assert!(hits.contains(&back), "hits were {hits:?}");
}
