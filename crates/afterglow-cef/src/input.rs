//! Native input capture for the CEF build.
//!
//! The host intercepts keyboard events (in `on_pre_key_event`, before the
//! page) and pushes them to a channel the game loop drains each frame. This is
//! the renderer-window -> game-loop input path on native, with **no web/JS
//! messages** — pure native, matching the worker-transport philosophy.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    KeyDown,
    KeyUp,
    Char,
}

#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub kind: InputKind,
    /// DOM/Windows key code (e.g. 65 = 'A', 37 = Left).
    pub key_code: i32,
    pub modifiers: u32,
}

static INPUT_TX: Mutex<Option<Sender<InputEvent>>> = Mutex::new(None);
static INPUT_RX: Mutex<Option<Receiver<InputEvent>>> = Mutex::new(None);

fn ensure_channel() {
    let mut tx = INPUT_TX.lock().expect("input tx lock");
    if tx.is_none() {
        let (s, r) = channel();
        *tx = Some(s);
        *INPUT_RX.lock().expect("input rx lock") = Some(r);
    }
}

/// Push a key event from the keyboard handler (UI thread) to the game loop.
pub(crate) fn push_input(ev: InputEvent) {
    ensure_channel();
    if let Some(tx) = INPUT_TX.lock().expect("input tx lock").as_ref() {
        let _ = tx.send(ev);
    }
}

/// Take the input receiver the game loop drains each frame. Returns `None`
/// after the first call. Non-blocking `try_recv` per frame.
pub fn take_input_receiver() -> Option<Receiver<InputEvent>> {
    ensure_channel();
    INPUT_RX.lock().expect("input rx lock").take()
}
