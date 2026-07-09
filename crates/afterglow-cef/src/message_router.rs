//! Simple custom page→host→page IPC via V8 + CEF process messages.
//!
//! Replaces the cef-rs message_router wrapper (whose IPC wasn't flowing) with
//! a direct, minimal implementation:
//! - Renderer: `on_context_created` registers `window.__afterglow_rpc(str)` as a
//!   V8 function. When called, it sends a `ProcessMessage("afterglow_rpc")` to
//!   the browser via `frame.send_process_message(BROWSER, msg)`.
//! - Browser: `Client::on_process_message_received` receives it, routes to a
//!   handler (echo for now → worker routing next), sends a response
//!   `ProcessMessage("afterglow_rpc_resp")` back to the renderer.
//! - Renderer: `on_process_message_received` receives the response and calls
//!   `window.__afterglow_rpc_resp(str)`.

use cef::*;
use std::sync::{Arc, Mutex, OnceLock};

/// Emulated Web Worker JS: wraps `__afterglow_rpc` (native CEF IPC) in a
/// `postMessage`/`onmessage` API identical to a real Web Worker.
/// On the web build, a real `Worker` is used instead.
pub const EMULATED_WORKER_JS: &str = r#"
if (!window.AfterglowWorker) {
  window.AfterglowWorker = class {
    constructor(name) {
      this._name = name;
      this.onmessage = null;
      this.onerror = null;
      window.__afterglow_rpc_resp = (resp) => {
        if (this.onmessage) this.onmessage({ data: resp });
      };
    }
    postMessage(msg) {
      window.__afterglow_rpc(this._name + '\u0000' + msg);
    }
    terminate() {}
  };
}
"#;

const RPC_MSG: &str = "afterglow_rpc";
const RPC_RESP: &str = "afterglow_rpc_resp";

/// Browser-side handler registry: `on_invoke` closure (set by AppBuilder).
pub static RPC_HANDLER: OnceLock<Arc<dyn Fn(&str) -> String + Send + Sync>> = OnceLock::new();

// --- V8 handler (renderer: page → host) -----------------------------------

wrap_v8_handler! {
    struct RpcV8Handler;

    impl V8Handler {
        fn execute(
            &self,
            name: Option<&CefString>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            retval: Option<&mut Option<V8Value>>,
            _exception: Option<&mut CefString>,
        ) -> i32 {
            let Some(name) = name else { return 0; };
            if name.to_string() != "__afterglow_rpc" { return 0; }

            // Get the request string from the first argument
            let request = arguments
                .and_then(|args| args.get(0).cloned().flatten())
                .filter(|v| v.is_string() != 0)
                .map(|v| CefString::from(&v.string_value()).to_string())
                .unwrap_or_default();

            eprintln!("[afterglow] V8 __afterglow_rpc: {request}");
            // Get the current V8 context → frame → send process message to browser
            let Some(ctx) = v8_context_get_current_context() else {
                eprintln!("[afterglow] V8: no current context");
                if let Some(r) = retval { *r = v8_value_create_int(0); }
                return 1;
            };
            let Some(frame) = ctx.frame() else {
                eprintln!("[afterglow] V8: no frame");
                if let Some(r) = retval { *r = v8_value_create_int(0); }
                return 1;
            };

            let msg_name = CefString::from(RPC_MSG);
            let Some(mut msg) = process_message_create(Some(&msg_name)) else {
                if let Some(r) = retval { *r = v8_value_create_int(0); }
                return 1;
            };
            if let Some(list) = msg.argument_list() {
                list.set_string(0, Some(&CefString::from(request.as_str())));
            }
            frame.send_process_message(ProcessId::BROWSER, Some(&mut msg));
            eprintln!("[afterglow] V8: sent");

            if let Some(r) = retval { *r = v8_value_create_int(1); }
            1
        }
    }
}

// --- RenderProcessHandler (renderer side) ---------------------------------

wrap_render_process_handler! {
    pub struct GameRenderProcessHandler;

    impl RenderProcessHandler {
        fn on_context_created(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            eprintln!("[afterglow] renderer registered __afterglow_rpc");
            let Some(ctx) = context else { return; };
            let Some(window) = ctx.global() else { return; };

            // Register window.__afterglow_rpc(str) → sends a process message to browser
            let name = CefString::from("__afterglow_rpc");
            let mut handler = RpcV8Handler::new();
            if let Some(mut func) = v8_value_create_function(Some(&name), Some(&mut handler)) {
                window.set_value_bykey(Some(&name), Some(&mut func), V8Propertyattribute::default());
                eprintln!("[afterglow] registered __afterglow_rpc");
            }
        }

        fn on_process_message_received(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _source_process: ProcessId, message: Option<&mut ProcessMessage>) -> i32 {
            let Some(msg) = message else { return 0; };
            let name = CefString::from(&msg.name()).to_string();

            if name == RPC_RESP {
                let resp = msg.argument_list()
                    .map(|l| CefString::from(&l.string(0)).to_string())
                    .unwrap_or_default();
                eprintln!("[afterglow] renderer response");
                if let Some(frame) = _frame {
                    let js = format!("window.__afterglow_rpc_resp && window.__afterglow_rpc_resp({:?});", resp);
                    frame.execute_java_script(
                        Some(&CefString::from(js.as_str())),
                        Some(&CefString::from("afterglow://rpc")),
                        0,
                    );
                    eprintln!("[afterglow] renderer JS callback");
                }
                return 1;
            }
            0
        }
    }
}

impl GameRenderProcessHandler {
    pub fn make() -> RenderProcessHandler {
        GameRenderProcessHandler::new()
    }
}

// --- Browser-side message handler (called from Client::on_process_message_received) ---

pub fn handle_browser_message(browser: Option<Browser>, frame: Option<Frame>, message: Option<ProcessMessage>) -> bool {
    let Some(msg) = message else { return false; };
    let name = CefString::from(&msg.name()).to_string();
    eprintln!("[afterglow] browser got message: {name}");

    if name == RPC_MSG {
        let request = msg.argument_list()
            .map(|l| CefString::from(&l.string(0)).to_string())
            .unwrap_or_default();
                eprintln!("[afterglow] browser RPC: {request}");

        // Route to handler (echo for now; worker routing next)
        let response = match RPC_HANDLER.get() {
            Some(h) => h(&request),
            None => request, // echo
        };

        // Send response back to renderer
        let resp_name = CefString::from(RPC_RESP);
        if let Some(mut resp_msg) = process_message_create(Some(&resp_name)) {
            if let Some(list) = resp_msg.argument_list() {
                list.set_string(0, Some(&CefString::from(response.as_str())));
            }
            if let Some(frame) = frame {
                frame.send_process_message(ProcessId::RENDERER, Some(&mut resp_msg));
                        eprintln!("[afterglow] browser sent response");
            }
        }
        return true;
    }
    false
}

// --- RequestHandler (browser-side lifecycle — empty for now) --------------

wrap_request_handler! {
    pub struct GameRequestHandler;

    impl RequestHandler {}
}
