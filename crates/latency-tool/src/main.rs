//! Automated input→present latency measurement for the cef-rs prototype.
//!
//! Connects to CEF's Chrome DevTools Protocol (browser-level endpoint), starts a
//! Chromium trace, then for N iterations: dispatches a synthetic mouse click via
//! CDP `Input.dispatchMouseEvent` and immediately drops a `performance.mark`
//! (a trace-clock marker) via `Runtime.evaluate`. After stopping the trace, it
//! parses each mark's timestamp vs the next `SkiaRenderer::SwapBuffers` (the
//! frame present) — all in the trace's own clock (no wall-clock alignment).
//!
//! What it measures: "input dispatched → next frame presented". CDP-dispatched
//! input bypasses the OS input stack, so this is a *lower bound* on true
//! input→present (OS→renderer needs real input / hardware). Reproducible, CI-able.
//!
//! Usage: latency-tool `[host:port]`   (default 127.0.0.1:9222)

use serde_json::{Value, json};
use std::time::Duration;
use tungstenite::Message;

const N: usize = 12;
type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

struct Cdp {
    ws: Ws,
    next_id: u32,
    session: String,
}

impl Cdp {
    fn connect(addr: &str) -> Self {
        let version: Value = ureq::get(&format!("http://{addr}/json/version"))
            .timeout(Duration::from_secs(3))
            .call()
            .expect("GET /json/version")
            .into_json()
            .expect("parse version");
        let ws_url = version["webSocketDebuggerUrl"]
            .as_str()
            .expect("webSocketDebuggerUrl");
        eprintln!("browser ws: {ws_url}");
        let (ws, _) = tungstenite::connect(ws_url).expect("ws connect");
        let mut this = Self {
            ws,
            next_id: 0,
            session: String::new(),
        };
        let targets = this.browser("Target.getTargets", json!({}));
        let page = targets["result"]["targetInfos"]
            .as_array()
            .expect("targetInfos")
            .iter()
            .find(|t| t["type"].as_str() == Some("page"))
            .expect("no page target");
        let target_id = page["targetId"].as_str().expect("targetId").to_owned();
        let attached = this.browser(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
        );
        this.session = attached["result"]["sessionId"]
            .as_str()
            .expect("sessionId")
            .to_owned();
        eprintln!("session: {}", this.session);
        this
    }

    fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    fn browser(&mut self, method: &str, params: Value) -> Value {
        let id = self.id();
        self.ws
            .send(Message::Text(
                json!({ "id": id, "method": method, "params": params }).to_string(),
            ))
            .unwrap();
        recv(&mut self.ws, id)
    }

    fn session(&mut self, method: &str, params: Value) -> Value {
        let id = self.id();
        self.ws
            .send(Message::Text(
                json!({ "id": id, "sessionId": self.session, "method": method, "params": params })
                    .to_string(),
            ))
            .unwrap();
        recv(&mut self.ws, id)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let eval_expr = if args.get(1).map(|s| s.as_str()) == Some("eval") {
        Some(
            args.get(2)
                .cloned()
                .expect("usage: latency-tool eval '<expr>'"),
        )
    } else if args.get(1).map(|s| s.as_str()) == Some("nav") {
        let url = args.get(2).cloned().expect("usage: latency-tool nav <url>");
        let addr = args
            .get(3)
            .cloned()
            .unwrap_or_else(|| "127.0.0.1:9222".into());
        let mut cdp = Cdp::connect(&addr);
        let _ = cdp.session("Network.enable", json!({}));
        let _ = cdp.session("Page.enable", json!({}));
        let r = cdp.session("Page.navigate", json!({"url": url}));
        println!("Page.navigate => {}", r["result"]);
        let deadline = std::time::Instant::now() + Duration::from_millis(2500);
        while std::time::Instant::now() < deadline {
            let Ok(msg) = cdp.ws.read() else { break };
            let Message::Text(t) = msg else { continue };
            let Ok(v) = serde_json::from_str::<Value>(&t) else {
                continue;
            };
            let m = v["method"].as_str().unwrap_or("");
            if m.starts_with("Network.loadingFailed") {
                println!("loadingFailed: {}", v["params"]);
            } else if m == "Network.responseReceived" {
                let p = &v["params"]["response"];
                println!(
                    "responseReceived: status={} mime={} url={}",
                    p["status"], p["mimeType"], p["url"]
                );
            } else if m == "Network.loadingFinished" {
                println!("loadingFinished: {}", v["params"]);
            } else if m == "Page.frameNavigated" {
                println!("frameNavigated: url={}", v["params"]["frame"]["url"]);
            }
        }
        return;
    } else {
        None
    };
    let addr = if eval_expr.is_some() {
        args.get(3).cloned()
    } else {
        args.get(1).cloned()
    }
    .unwrap_or_else(|| "127.0.0.1:9222".into());
    let mut cdp = Cdp::connect(&addr);

    if let Some(expr) = eval_expr {
        let r = cdp.session(
            "Runtime.evaluate",
            json!({ "expression": expr, "returnByValue": true, "awaitPromise": true }),
        );
        if let Some(exc) = r["result"]["exceptionDetails"].as_object() {
            println!(
                "EXCEPTION: {}",
                exc.get("exception")
                    .and_then(|e| e["description"].as_str())
                    .unwrap_or("(no desc)")
            );
        } else {
            println!("=> {}", r["result"]["result"]["value"]);
        }
        return;
    }

    // Start trace (compositor+gpu for SwapBuffers; blink for performance marks).
    let cats = "blink,cc,gpu,v8,benchmark,devtools.timeline,disabled-by-default-devtools.timeline,user_timing";
    let _ = cdp.browser(
        "Tracing.start",
        json!({ "traceConfig": { "includedCategories": cats.split(',').collect::<Vec<_>>() } }),
    );

    // N iterations: dispatch a click, then drop a trace-clock marker via
    // Tracing.recordClockSyncMarker (bulletproof; performance.mark via JS
    // turned out not to be captured in this CEF build).
    std::thread::sleep(Duration::from_millis(150));
    for i in 0..N {
        for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
            let _ = cdp.session(
                "Input.dispatchMouseEvent",
                json!({
                    "type": kind, "x": 640.0, "y": 400.0, "button": "left", "clickCount": 1
                }),
            );
        }
        let _ = cdp.browser(
            "Tracing.recordClockSyncMarker",
            json!({ "syncId": format!("ag_{i}") }),
        );
        std::thread::sleep(Duration::from_millis(120));
    }
    std::thread::sleep(Duration::from_millis(200));

    // End trace and drain.
    let end_id = cdp.id();
    cdp.ws
        .send(Message::Text(
            json!({ "id": end_id, "method": "Tracing.end", "params": {} }).to_string(),
        ))
        .unwrap();
    let mut events: Vec<Value> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        let Ok(msg) = cdp.ws.read() else { break };
        let Message::Text(t) = msg else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&t) else {
            continue;
        };
        if v["method"].as_str() == Some("Tracing.dataCollected") {
            if let Some(a) = v["params"]["value"].as_array() {
                events.extend(a.iter().cloned());
            }
        } else if v["method"].as_str() == Some("Tracing.tracingComplete") {
            break;
        }
    }
    eprintln!("collected {} trace events", events.len());
    report(&events);
}

fn recv(ws: &mut Ws, id: u32) -> Value {
    loop {
        let msg = ws.read().unwrap();
        if let Message::Text(t) = msg {
            let v: Value = serde_json::from_str(&t).unwrap();
            if v["id"].as_u64() == Some(id as u64) {
                return v;
            }
        }
    }
}

fn is_input(name: &str) -> bool {
    name == "EventDispatch"
        || name.contains("MousePress")
        || name.contains("MouseRelease")
        || name.contains("MouseMove")
        || name.contains("OnHandleInputEvent")
}

fn collect_samples(events: &[Value]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut swaps: Vec<f64> = events
        .iter()
        .filter(|e| e["name"].as_str() == Some("SkiaRenderer::SwapBuffers"))
        .filter_map(|e| e["ts"].as_f64())
        .collect();
    swaps.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let mut markers: Vec<f64> = events
        .iter()
        .filter_map(|e| {
            e["name"]
                .as_str()
                .filter(|n| is_input(n))
                .and_then(|_| e["ts"].as_f64())
        })
        .collect();
    markers.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let mut samples: Vec<f64> = Vec::new();
    let mut prev = 0.0f64;
    for t0 in &markers {
        // dedupe markers closer than 1ms (one input burst fires several events)
        if *t0 - prev < 1000.0 {
            continue;
        }
        prev = *t0;
        if let Some(t1) = swaps.iter().find(|t| **t > *t0).copied() {
            samples.push((t1 - t0) / 1000.0); // µs -> ms
        }
    }
    (markers, swaps, samples)
}

fn report(events: &[Value]) {
    let (markers, swaps, samples) = collect_samples(events);
    eprintln!(
        "markers={} swaps={} samples={}",
        markers.len(),
        swaps.len(),
        samples.len()
    );

    println!(
        "=== input(mark) → next SkiaRenderer::SwapBuffers (present), {}/{} samples ===",
        samples.len(),
        N
    );
    if samples.is_empty() {
        println!("(no samples — dumping trace event names matching input/clock/sync/present):");
        let mut seen: std::collections::HashMap<String, u32> = Default::default();
        for e in events {
            let Some(n) = e["name"].as_str() else {
                continue;
            };
            if n.contains("Swap")
                || n.contains("Present")
                || n.contains("Input")
                || n.contains("Event")
                || n.contains("Mouse")
                || n.contains("clock")
                || n.contains("Sync")
                || n.contains("ag_")
                || n.contains("Mark")
            {
                *seen.entry(n.to_string()).or_default() += 1;
            }
        }
        let mut v: Vec<_> = seen.into_iter().collect();
        v.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (k, c) in v {
            println!("  {k}: {c}");
        }
        return;
    }
    let mut s = samples.clone();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = s.iter().sum::<f64>() / s.len() as f64;
    let pct = |p: f64| s[(s.len() as f64 * p).floor() as usize];
    println!(
        "  samples: {:?}",
        samples
            .iter()
            .map(|x| format!("{x:.2}"))
            .collect::<Vec<_>>()
    );
    println!(
        "  min={:.2}  median={:.2}  mean={:.2}  p90={:.2}  max={:.2}  (ms)",
        s[0],
        pct(0.5),
        mean,
        pct(0.9),
        s[s.len() - 1]
    );

    // Present rate: inter-swap intervals -> fps.
    if swaps.len() > 1 {
        let mut intervals = Vec::<f64>::new();
        for w in swaps.windows(2) {
            intervals.push((w[1] - w[0]) / 1000.0);
        }
        intervals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean_int = intervals.iter().sum::<f64>() / intervals.len() as f64;
        let med_int = intervals[intervals.len() / 2];
        println!("\n=== present rate (SkiaRenderer::SwapBuffers cadence) ===");
        println!(
            "  swaps={}  mean interval={:.2}ms ({:.0} fps)  median interval={:.2}ms ({:.0} fps)",
            swaps.len(),
            mean_int,
            1000.0 / mean_int,
            med_int,
            1000.0 / med_int
        );
        let p1 = intervals[(intervals.len() as f64 * 0.01) as usize];
        let p99 = intervals[(intervals.len() as f64 * 0.99).floor() as usize];
        let p999 = intervals[(intervals.len() as f64 * 0.999).floor() as usize];
        println!(
            "  interval min={:.2}ms  p1={:.2}ms  median={:.2}ms  p99={:.2}ms ({:.0} fps)  p99.9={:.2}ms  max={:.2}ms",
            intervals[0],
            p1,
            med_int,
            p99,
            1000.0 / p99,
            p999,
            intervals[intervals.len() - 1]
        );
        // Count frames below 55 FPS (>18.18ms interval).
        let below_55 = intervals.iter().filter(|&&i| i > 18.18).count();
        let below_30 = intervals.iter().filter(|&&i| i > 33.33).count();
        println!(
            "  frames < 55 FPS: {} / {} ({:.1}%)   frames < 30 FPS: {}",
            below_55,
            intervals.len(),
            100.0 * below_55 as f64 / intervals.len() as f64,
            below_30
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_events_pair_each_input_burst_with_next_present() {
        let events = vec![
            json!({"name":"EventDispatch", "ts":1000.0}),
            json!({"name":"MousePress", "ts":1100.0}),
            json!({"name":"SkiaRenderer::SwapBuffers", "ts":2500.0}),
            json!({"name":"MouseMove", "ts":4000.0}),
            json!({"name":"SkiaRenderer::SwapBuffers", "ts":7000.0}),
        ];
        let (markers, swaps, samples) = collect_samples(&events);
        assert_eq!(markers.len(), 3);
        assert_eq!(swaps, vec![2500.0, 7000.0]);
        assert_eq!(samples, vec![1.5, 3.0]);
    }
}
