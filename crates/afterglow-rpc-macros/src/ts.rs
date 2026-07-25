//! TypeScript client generation for `#[rpc(worker = ...)]` traits.
//!
//! When the macro detects `worker = Type`, it generates a typed `.ts` client
//! alongside the Rust client + wasm exports. The generated client wraps the
//! low-level byte `rpc.call(methodId, args)` with typed methods and postcard
//! codecs for the specific types on the trait (not arbitrary serde).
//!
//! Supported types: f32, f64, u8, u16, u32, u64, usize, i8, i16, i32, i64,
//! isize, bool, String, Vec<u8>, Vec<f32>, Vec<f64>, and `()`. Other types
//! produce a compile error pointing the user to `#[derive(AfterglowTs)]`
//! (future) or primitives.

use std::path::Path;

use quote::ToTokens;
use syn::{FnArg, Ident, ItemTrait, Pat, TraitItem, Type, TypePath};

/// TS type info for a Rust type appearing in an `#[rpc]` signature.
struct TsType {
    /// TS type string (e.g. `"number"`, `"Float32Array"`).
    ts: String,
    /// Name of the encode function in `codec.ts` (e.g. `"encodeF32"`).
    encode_fn: &'static str,
    /// Name of the decode function in `codec.ts` (e.g. `"decodeF32"`).
    decode_fn: &'static str,
}

/// Translate a Rust type to TS type info.
fn translate(ty: &Type) -> syn::Result<TsType> {
    let Type::Path(p) = ty else {
        return Err(err(ty, "TS client: only path types are supported"));
    };
    let ident = last_ident(p);
    match ident.to_string().as_str() {
        "f32" => ok("number", "encodeF32", "decodeF32"),
        "f64" => ok("number", "encodeF64", "decodeF64"),
        "bool" => ok("boolean", "encodeBool", "decodeBool"),
        "u8" => ok("number", "encodeU8", "decodeU8"),
        "u16" => ok("number", "encodeU16", "decodeU16"),
        "u32" => ok("number", "encodeU32", "decodeU32"),
        "u64" | "usize" => ok("number", "encodeU64", "decodeU64"),
        "i8" => ok("number", "encodeI8", "decodeI8"),
        "i16" => ok("number", "encodeI16", "decodeI16"),
        "i32" => ok("number", "encodeI32", "decodeI32"),
        "i64" | "isize" => ok("number", "encodeI64", "decodeI64"),
        "String" => ok("string", "encodeString", "decodeString"),
        "Vec" => {
            let inner = first_generic_arg(p)?;
            let inner_ident = type_ident(inner)?;
            match inner_ident.to_string().as_str() {
                "u8" => ok("Uint8Array", "encodeBytes", "decodeBytes"),
                "u32" => ok("Uint32Array", "encodeU32Vec", "decodeU32Vec"),
                "f32" => ok("Float32Array", "encodeF32Vec", "decodeF32Vec"),
                "f64" => ok("Float64Array", "encodeF64Vec", "decodeF64Vec"),
                other => Err(err(
                    inner,
                    format!(
                        "TS client: Vec<{other}> not supported; use Vec<u8>, Vec<f32>, or Vec<f64>"
                    ),
                )),
            }
        }
        // RpcResult<T> / Result<T, RpcError> — in TS, errors are thrown,
        // so the return type maps to just T (the success type).
        "RpcResult" | "Result" => {
            let inner = first_generic_arg(p)?;
            if is_unit(inner) {
                return Ok(TsType {
                    ts: "void".to_string(),
                    encode_fn: "",
                    decode_fn: "",
                });
            }
            translate(inner)
        }
        other => Err(err(
            ty,
            format!(
                "TS client: type `{other}` not supported; use primitives, String, Vec<u8>/Vec<f32>/Vec<f64>, or annotate the type with #[derive(AfterglowTs)]"
            ),
        )),
    }
}

/// Translate the return type, mapping `()` to the unit TS type and
/// `RpcResult<T>`/`Result<T, _>` to `T` (errors are thrown in TS).
fn translate_ret(ty: &Type) -> syn::Result<TsType> {
    if is_unit(ty) {
        return Ok(TsType {
            ts: "void".to_string(),
            encode_fn: "",
            decode_fn: "",
        });
    }
    translate(ty)
}

fn is_unit(ty: &Type) -> bool {
    if let Type::Tuple(t) = ty {
        return t.elems.is_empty();
    }
    false
}

// --- helpers ------------------------------------------------------------

fn ok(ts: &str, encode_fn: &'static str, decode_fn: &'static str) -> syn::Result<TsType> {
    Ok(TsType {
        ts: ts.to_string(),
        encode_fn,
        decode_fn,
    })
}

fn err(span: &dyn ToTokens, msg: impl Into<String>) -> syn::Error {
    syn::Error::new_spanned(span, msg.into())
}

/// Last segment identifier of a `TypePath` (e.g. `std::vec::Vec` → `Vec`).
fn last_ident(p: &TypePath) -> &Ident {
    &p.path.segments.last().unwrap().ident
}

/// Extract the last path segment identifier from a `Type` (must be `Type::Path`).
fn type_ident(ty: &Type) -> syn::Result<&Ident> {
    match ty {
        Type::Path(p) => Ok(last_ident(p)),
        _ => Err(err(ty, "TS client: expected a path type")),
    }
}

/// First generic type argument of a path's last segment.
fn first_generic_arg(p: &TypePath) -> syn::Result<&Type> {
    let seg = p.path.segments.last().unwrap();
    let args = &seg.arguments;
    let syn::PathArguments::AngleBracketed(ab) = args else {
        return Err(err(seg, "TS client: expected generic type arguments"));
    };
    let Some(syn::GenericArgument::Type(t)) = ab.args.first() else {
        return Err(err(seg, "TS client: expected a type argument"));
    };
    Ok(t)
}

/// Convert a Rust `snake_case` identifier to TS `camelCase`.
fn camel(ident: &Ident) -> String {
    let s = ident.to_string();
    let mut out = String::with_capacity(s.len());
    let mut up = false;
    for c in s.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.push(c.to_ascii_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

// --- client generation --------------------------------------------------

/// Generate the TypeScript client source for a trait.
///
/// `is_async` controls the `spawn()` method: async workers use `AsyncWorker`
/// (poll model), sync workers use `Rpc` (blocking ring transport).
pub fn generate_client(tr: &ItemTrait, is_async: bool) -> syn::Result<String> {
    let name = &tr.ident;
    let client_name = format!("{name}Client");
    let mut methods = Vec::new();
    let mut imports = std::collections::BTreeSet::new();
    imports.insert("concat".to_string());

    for (id, item) in tr.items.iter().enumerate() {
        let TraitItem::Fn(m) = item else { continue };
        let sig = &m.sig;
        let mname = camel(&sig.ident);
        let id_lit = id as u32;

        // Translate params.
        let mut params_ts = Vec::new();
        let mut encode_args = Vec::new();
        for a in &sig.inputs {
            let FnArg::Typed(pt) = a else { continue };
            let Pat::Ident(pi) = &*pt.pat else { continue };
            let pname = camel(&pi.ident);
            let ty = translate(&pt.ty)?;
            params_ts.push(format!("{pname}: {ts}", ts = ty.ts));
            if !ty.encode_fn.is_empty() {
                imports.insert(ty.encode_fn.to_string());
                encode_args.push(format!("{enc}({pname})", enc = ty.encode_fn));
            }
        }

        // Translate return.
        let ret = match &sig.output {
            syn::ReturnType::Type(_, t) => translate_ret(t)?,
            syn::ReturnType::Default => TsType {
                ts: "void".to_string(),
                encode_fn: "",
                decode_fn: "",
            },
        };

        // Build the args expression.
        let args_expr = if encode_args.is_empty() {
            "new Uint8Array()".to_string()
        } else if encode_args.len() == 1 {
            encode_args[0].clone()
        } else {
            format!("concat({})", encode_args.join(", "))
        };

        let params = params_ts.join(", ");
        let body = if ret.decode_fn.is_empty() {
            vec![
                format!("  async {mname}({params}): Promise<void> {{"),
                format!("    const args = {args_expr};"),
                format!("    await this.rpc.call({id_lit}, args);"),
                "  }".to_string(),
            ]
            .join("\n")
        } else {
            imports.insert(ret.decode_fn.to_string());
            vec![
                format!(
                    "  async {mname}({params}): Promise<{ret_ts}> {{",
                    ret_ts = ret.ts
                ),
                format!("    const args = {args_expr};"),
                format!("    const resp = await this.rpc.call({id_lit}, args);"),
                format!("    return {dec}(resp, 0)[0];", dec = ret.decode_fn),
                "  }".to_string(),
            ]
            .join("\n")
        };
        methods.push(body);
    }

    let import_list: Vec<String> = imports.iter().cloned().collect();
    let mut lines = vec![
        "// AUTO-GENERATED by #[rpc]. Do not edit.".to_string(),
        format!("// Source trait: {name}"),
        format!(
            "import {{ RpcTransport, {imports} }} from './codec.ts';",
            imports = import_list.join(", ")
        ),
    ];

    if is_async {
        lines.push(
            "import { AsyncWorker, asyncWorkerImports } from './async-worker.ts';".to_string(),
        );
        lines.push("import { Rpc } from './rpc.ts';".to_string());
    } else {
        lines.push("import { Rpc } from './rpc.ts';".to_string());
    }

    lines.push(String::new());
    lines.push(format!("export class {client_name} {{"));
    lines.push("  private rpc: RpcTransport;".to_string());
    lines.push("  private closed = false;".to_string());
    lines.push(String::new());

    if is_async {
        // Async: static spawn() that instantiates the wasm + returns the client.
        // Also a poll() method that drives the executor + resolves promises.
        // The wasm URL defaults to `<name>.wasm` (lowercased trait name) — the
        // user doesn't need to pass it unless they renamed the file.
        let wasm_default = format!("{}.wasm", name.to_string().to_lowercase());
        lines.extend([
            format!("  /// Spawn the async worker. Instantiates the wasm module, wires the"),
            format!("  /// fetch imports, and returns a ready-to-use client. Call poll()"),
            format!("  /// each frame to drive the executor + resolve pending promises."),
            format!("  static async spawn(workerWasmUrl = '{wasm_default}', baseUrl = ''): Promise<{client_name}> {{"),
            format!("    const driver = new AsyncWorker(null, baseUrl);"),
            format!("    const memory = new WebAssembly.Memory({{ shared: true, initial: 256, maximum: 1024 }});"),
            format!("    const {{ instance }} = await WebAssembly.instantiate("),
            format!("      await (await fetch(workerWasmUrl)).arrayBuffer(),"),
            format!("      asyncWorkerImports(driver, memory),"),
            format!("    );"),
            format!("    driver.w = instance.exports;"),
            format!("    instance.exports.afterglow_wasm_init();"),
            format!("    return new {client_name}(driver);"),
            format!("  }}"),
            String::new(),
            format!("  /// Spawn the service in a real Web Worker using the shared-ring transport."),
            format!("  static async spawnThreaded(opts: {{ mainWasmUrl?: string; workerJsUrl?: string; workerWasmUrl?: string; timeoutMs?: number }} = {{}}): Promise<{client_name}> {{"),
            format!("    const rpc = await Rpc.create({{"),
            format!("      mainWasmUrl: opts.mainWasmUrl ?? 'afterglow_rpc.wasm',"),
            format!("      workerJsUrl: opts.workerJsUrl ?? 'worker.js',"),
            format!("      workerWasmUrl: opts.workerWasmUrl ?? '{wasm_default}',"),
            format!("      timeoutMs: opts.timeoutMs,"),
            format!("    }});"),
            format!("    return new {client_name}(rpc);"),
            format!("  }}"),
            String::new(),
            format!("  /// Drive a locally instantiated async service. Threaded clients do not require polling."),
            format!("  poll(): void {{ this.rpc.poll?.(); }}"),
        ]);
    } else {
        // Sync: static spawn() that uses Rpc.create (the blocking ring transport).
        let wasm_default = format!("{}.wasm", name.to_string().to_lowercase());
        lines.extend([
            format!("  /// Spawn the sync worker. Instantiates the shared wasm + worker,"),
            format!("  /// and returns a ready-to-use client."),
            format!("  static async spawn(opts: {{ mainWasmUrl?: string; workerJsUrl?: string; workerWasmUrl?: string; timeoutMs?: number }} = {{}}): Promise<{client_name}> {{"),
            format!("    const rpc = await Rpc.create({{"),
            format!("      mainWasmUrl: opts.mainWasmUrl ?? 'afterglow_rpc.wasm',"),
            format!("      workerJsUrl: opts.workerJsUrl ?? 'worker.js',"),
            format!("      workerWasmUrl: opts.workerWasmUrl ?? '{wasm_default}',"),
            format!("      timeoutMs: opts.timeoutMs,"),
            format!("    }});"),
            format!("    return new {client_name}(rpc);"),
            format!("  }}"),
        ]);
    }

    lines.push(String::new());
    lines.push(format!(
        "  constructor(rpc: RpcTransport) {{ this.rpc = rpc; }}"
    ));
    lines.push(String::new());
    lines.push("  /// Idempotently stop an owned worker transport when supported.".to_string());
    lines.push(
        "  close(): void { if (this.closed) return; this.closed = true; this.rpc.terminate?.(); }"
            .to_string(),
    );
    if !methods.is_empty() {
        lines.push(String::new());
        lines.push(methods.join("\n\n"));
    }
    lines.push("}".to_string());
    lines.push(String::new());
    Ok(lines.join("\n"))
}

/// Write the generated TS client to `CARGO_MANIFEST_DIR/ts/<name>.client.ts`.
/// Idempotent: only writes if content changed. Does nothing if
/// `CARGO_MANIFEST_DIR` is unset (e.g. some IDE check contexts).
pub fn write_client(name: &str, source: &str) {
    let Some(dir) = std::env::var_os("CARGO_MANIFEST_DIR") else {
        return;
    };
    let ts_dir = Path::new(&dir).join("gen");
    let _ = std::fs::create_dir_all(&ts_dir);
    let file = ts_dir.join(format!("{}.client.ts", name.to_lowercase()));
    // Idempotent: skip write if unchanged (avoids touching mtimes).
    if let Ok(existing) = std::fs::read_to_string(&file) {
        if existing == source {
            return;
        }
    }
    let _ = std::fs::write(&file, source);
    // Tell cargo to watch this path so changes re-trigger.
    // (proc_macro::tracked_path::path is unstable; the crate recompile on
    // trait change is sufficient since the macro re-expands.)
}

/// Entry point called from the `#[rpc]` macro when `worker = Type` is set.
/// Generates the TS client and writes it to disk. `is_async` controls whether
/// the generated `spawn()` uses the poll model (`AsyncWorker`) or the blocking
/// ring transport (`Rpc`).
pub fn generate_and_write(tr: &ItemTrait, is_async: bool) -> syn::Result<()> {
    let name = &tr.ident;
    let src = generate_client(tr, is_async)?;
    write_client(&name.to_string(), &src);
    Ok(())
}
