import { runInDevShell, inNixShell } from "../utils/nix";
import { requireTools } from "../utils/detect";

const FLAGS = "--profile wasm-release --target wasm32-unknown-unknown -p agx";
const ROOT = new URL("../../", import.meta.url).pathname;
const WASM = new URL("../../wasm/", import.meta.url).pathname;

let building = false;

async function hasWasmBindgen(): Promise<boolean> {
  const r = Bun.spawnSync(["which", "wasm-bindgen"]);
  return r.exitCode === 0;
}

async function build(): Promise<boolean> {
  if (building) return false;
  building = true;
  console.log("\n  [afterglow] rebuilding...");
  const cmd = [
    `cd "${ROOT}"`,
    `cargo build ${FLAGS}`,
    `wasm-bindgen --out-dir wasm --web target/wasm32-unknown-unknown/wasm-release/agx.wasm`,
  ].join(" && ");
  const code = await runInDevShell(cmd);
  console.log(`  [afterglow] ${code === 0 ? "done" : "failed"}`);
  building = false;
  return code === 0;
}

const RELOAD_SCRIPT = `
<script>(function(){var s=new EventSource("/__lr");s.addEventListener("reload",function(){s.close();location.reload()});s.addEventListener("error",function(){s.close()})})();<\/script>`;

export async function devServe(): Promise<void> {
  await requireTools(["cargo"], "Run `nix develop` or add to NixOS packages");

  const inShell = await inNixShell();
  const hasWbg = await hasWasmBindgen();

  if (!hasWbg && !inShell) {
    // Enter nix shell provides wasm-bindgen, handled by runInDevShell inside build()
    console.log("  [afterglow] will use nix shell for wasm-bindgen");
  } else if (!hasWbg) {
    console.error("\n  Missing tool: wasm-bindgen");
    console.error("  Ensure your flake/nix-shell provides wasm-bindgen");
    process.exit(1);
  }

  const port = parseInt(process.env.PORT || "4000", 10);
  await build();

  // Watch crates/ for .rs changes via polling (no external deps needed)
  const { readdirSync, statSync } = await import("fs");
  const { resolve } = await import("path");

  let lastMtime = 0;
  async function scanMtime(): Promise<number> {
    let latest = lastMtime;
    function walk(dir: string) {
      try {
        for (const name of readdirSync(dir)) {
          if (name === "target" || name.startsWith(".")) continue;
          const full = resolve(dir, name);
          try {
            const s = statSync(full);
            if (s.isDirectory()) walk(full);
            else if (name.endsWith(".rs") && s.mtimeMs > latest) latest = s.mtimeMs;
          } catch {}
        }
      } catch {}
    }
    walk(resolve(ROOT, "crates"));
    return latest;
  }

  lastMtime = await scanMtime();
  setInterval(async () => {
    const mtime = await scanMtime();
    if (mtime > lastMtime) {
      lastMtime = mtime;
      const ok = await build();
      if (ok) notify();
    }
  }, 500);

  let notify = () => {};
  const server = Bun.serve({
    port,
    async fetch(req) {
      const url = new URL(req.url);

      if (url.pathname === "/__lr") {
        let closed = false;
        return new Response(new ReadableStream({
          start(controller) {
            controller.enqueue("data: connected\n\n");
            notify = () => {
              if (!closed) { controller.enqueue("event: reload\ndata:\n\n"); controller.close(); closed = true; }
            };
            req.signal?.addEventListener("abort", () => { closed = true; });
          },
        }), {
          headers: { "Content-Type": "text/event-stream", "Cache-Control": "no-cache", "Access-Control-Allow-Origin": "*" },
        });
      }

      let filePath = url.pathname === "/" ? "/index.html" : url.pathname;
      const fullPath = WASM + filePath.slice(1);
      const file = Bun.file(fullPath);
      if (await file.exists()) {
        if (filePath.endsWith(".html")) {
          let html = await file.text();
          html = html.replace("</body>", RELOAD_SCRIPT + "\n</body>");
          return new Response(html, { headers: { "Content-Type": "text/html" } });
        }
        return new Response(file);
      }
      return new Response("not found", { status: 404 });
    },
  });

  console.log(`  [afterglow] → http://localhost:${port}`);
  await new Promise(() => {});
}
