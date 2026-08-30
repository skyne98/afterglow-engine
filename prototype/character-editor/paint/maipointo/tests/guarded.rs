#![cfg(feature = "demo")]
use maipointo::app::PaintApp;

/// The history byte budget must evict deterministically instead of OOM:
/// large strokes on a 16K app each capture hundreds of tiles; without the
/// budget the history Vecs grow until the module OOM-aborts.
#[test]
fn history_budget_evicts() {
    let mut app = PaintApp::new(16384, 16384).unwrap();
    app.set_background_color(0.659, 0.643, 0.596);
    for stroke in 0..12u32 {
        app.begin_stroke(100.0 + stroke as f32 * 20.0, 100.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        app.active().begin_atomic();
        for i in 0..60 {
            let _ = app.stroke_to(
                100.0 + stroke as f32 * 20.0 + i as f32 * 40.0, 100.0,
                0.7, 0.0, 0.0, 0.016, 1.0, 0.0, 0.0, false,
            );
        }
        app.active().end_atomic();
        app.history_commit();
        assert!(app.history_can_undo());
        assert!(app.history_entry_bytes() <= maipointo::app::HISTORY_BYTE_BUDGET
            + 4 * 1024 * 1024);
    }
    // Undo/redo stays consistent across the evictions.
    assert!(app.history_undo());
    assert!(app.history_redo());
}
