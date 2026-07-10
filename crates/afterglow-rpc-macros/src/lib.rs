//! `#[rpc]` on a trait generates:
//! - `<Name>Server` — the trait a worker implements, with a **provided**
//!   `serve(&mut self, method, args) -> RpcResult<Vec<u8>>` dispatch (no free
//!   `serve` fn, so two traits in one module don't collide).
//! - `<Name>Client<T: Transport>` — a typed client with per-method wrappers
//!   and a `transport(&self) -> &T` accessor (fields stay private).
//! - `<NAME>_SCHEMA: &RpcSchema` — a service-prefixed schema static.
//! - With `#[rpc(worker = Type)]`: `spawn_worker` (native) + the wasm exports
//!   `afterglow_wasm_init` / `afterglow_wasm_serve_frame` (+ scratch ptr/size
//!   exports) used by `afterglow-web`'s `worker.js`.
//!
//! ## Supported trait form
//! ```ignore
//! #[rpc(worker = PhysicsWorker)]
//! pub trait Physics {
//!     fn step(state: Vec<f32>, dt: f32) -> Vec<f32>;
//! }
//! ```
//! Methods must be plain `fn name(ident: Type, ...) -> Type;` — no receiver,
//! no trait generics, no supertraits, no default bodies, no
//! `async`/`const`/`unsafe`/`extern`, no associated consts/types, and
//! parameters must be simple identifiers. The macro injects `&mut self` into
//! the generated `<Name>Server` methods and preserves the trait's visibility.
//!
//! Method names must not collide with the generated API: `serve`, `new`, and
//! `transport` are always reserved, and `spawn_worker` is reserved only when
//! `#[rpc(worker = ...)]` is used (it generates the native client constructor).
//!
//! ## Limitation
//! The `#[no_mangle]` wasm exports use fixed names (`afterglow_wasm_*`), so at
//! most one `#[rpc(worker = ...)]` service may be linked into a single wasm
//! `cdylib`. Multiple non-worker `#[rpc]` traits may coexist in one module.

use proc_macro::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{FnArg, Ident, ItemTrait, Pat, TraitItem, Type};

/// Parsed `#[rpc]` attribute: empty or `worker = <Type>`.
struct RpcAttr {
    worker: Option<Type>,
}

impl syn::parse::Parse for RpcAttr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self { worker: None });
        }
        let ident: Ident = input.parse()?;
        if ident != "worker" {
            return Err(syn::Error::new(
                ident.span(),
                "unsupported #[rpc] attribute; expected `worker = Type` or empty",
            ));
        }
        let _: syn::Token![=] = input.parse()?;
        let ty: Type = input.parse()?;
        if !input.is_empty() {
            return Err(syn::Error::new(
                input.span(),
                "unexpected tokens after `worker = Type`",
            ));
        }
        Ok(Self { worker: Some(ty) })
    }
}

#[proc_macro_attribute]
pub fn rpc(attr: TokenStream, item: TokenStream) -> TokenStream {
    let worker = match syn::parse::<RpcAttr>(attr) {
        Ok(a) => a.worker,
        Err(e) => return e.to_compile_error().into(),
    };
    let tr = syn::parse_macro_input!(item as ItemTrait);
    if let Err(e) = validate_trait(&tr, worker.as_ref()) {
        return e.to_compile_error().into();
    }

    let vis = &tr.vis;
    let name = &tr.ident;
    let server = format_ident!("{}Server", name);
    let client = format_ident!("{}Client", name);
    let schema = format_ident!("{}_SCHEMA", name.to_string().to_uppercase());
    let name_str = name.to_string();

    let mut sigs = Vec::new();
    let mut clients = Vec::new();
    let mut arms = Vec::new();
    let mut methods = Vec::new();

    for (id, item) in tr.items.iter().enumerate() {
        let TraitItem::Fn(m) = item else { continue }; // validate_trait rejected the rest
        let sig = &m.sig;
        let mname = &sig.ident;
        let mname_str = mname.to_string();
        let id_lit = id as u32;

        let mut pats = Vec::new();
        let mut tys = Vec::new();
        let mut ppairs = Vec::new();
        for a in &sig.inputs {
            let FnArg::Typed(pt) = a else { continue }; // receivers rejected
            let Pat::Ident(pi) = &*pt.pat else { continue }; // idents only
            let pat = &pi.ident;
            let ty = &pt.ty;
            pats.push(quote! { #pat });
            tys.push(quote! { #ty });
            ppairs.push((
                pat.to_string(),
                ty.to_token_stream().to_string().replace(' ', ""),
            ));
        }
        let ret: Type = match &sig.output {
            syn::ReturnType::Type(_, t) => (**t).clone(),
            _ => syn::parse_quote!(()),
        };
        let ret_str = ret.to_token_stream().to_string().replace(' ', "");

        // Trailing commas force tuple semantics for the 1-param case (`(a,)`),
        // so single-arg methods round-trip like multi-arg ones.
        sigs.push(quote! { fn #mname(&mut self, #( #pats: #tys ),*) -> #ret; });
        clients.push(quote! {
            pub fn #mname(&self, #( #pats: #tys ),*) -> ::afterglow_rpc::RpcResult<#ret> {
                let resp = self.t.call(#name_str, #id_lit, &::afterglow_rpc::encode(&(#( #pats, )*))?)?;
                ::afterglow_rpc::decode(&resp)
            }
        });
        arms.push(quote! {
            #id_lit => {
                let (#( #pats, )*): (#( #tys, )*) = ::afterglow_rpc::decode(args)?;
                ::afterglow_rpc::encode(&self.#mname(#( #pats ),*))
            }
        });
        let pp = ppairs.iter().map(|(p, t)| quote! { (#p, #t) });
        methods.push(quote! {
            ::afterglow_rpc::RpcMethod { id: #id_lit, name: #mname_str, params: &[#( #pp ),*], returns: #ret_str }
        });
    }

    let (spawn, wasm) = match &worker {
        Some(wt) => {
            let spawn = quote! {
                #[cfg(not(target_arch = "wasm32"))]
                impl #client<::afterglow_rpc::native::WorkerTransport> {
                    /// Spawn the worker as a native OS thread over heap-backed ring
                    /// buffers. Returns the typed client + event receiver. The worker
                    /// thread is joined when the client is dropped.
                    pub fn spawn_worker(impl_: #wt)
                        -> ::afterglow_rpc::RpcResult<(Self, ::afterglow_rpc::native::EventReceiver)>
                    {
                        let serve = |s: &mut #wt, m: u32, a: &[u8]| s.serve(m, a);
                        let (t, ev) = ::afterglow_rpc::native::spawn_worker_loop(impl_, 1 << 20, serve)?;
                        Ok((Self::new(t), ev))
                    }
                }
            };
            let wasm = quote! {
                // Web worker exports in a private `#[cfg(wasm32)]` module, so none
                // of this is compiled natively (where multiple worker traits may
                // coexist). On wasm at most one worker service is allowed.
                #[cfg(target_arch = "wasm32")]
                mod afterglow_wasm {
                    use super::{#server, #wt};
                    use std::cell::{RefCell, UnsafeCell};

                    const IN_SIZE: usize = 1 << 20;
                    const OUT_SIZE: usize = 1 << 20;

                    #[repr(C, align(4))]
                    struct Scratch<const N: usize>(UnsafeCell<[u8; N]>);
                    unsafe impl<const N: usize> Sync for Scratch<N> {}

                    static INPUT: Scratch<{ IN_SIZE }> = Scratch(UnsafeCell::new([0; IN_SIZE]));
                    static OUTPUT: Scratch<{ OUT_SIZE }> = Scratch(UnsafeCell::new([0; OUT_SIZE]));

                    thread_local! {
                        static WORKER: RefCell<Option<Box<dyn #server>>> = const { RefCell::new(None) };
                    }

                    /// Construct the worker impl. JS calls this once after
                    /// instantiating the module. Requires `#wt: Default`.
                    #[unsafe(no_mangle)]
                    pub extern "C" fn afterglow_wasm_init() {
                        WORKER.with(|w| *w.borrow_mut() = Some(Box::new(#wt::default())));
                    }

                    /// # Safety
                    /// `args_ptr` must point to `args_len` readable bytes and
                    /// `out_ptr` to `out_max` writable bytes in wasm linear memory,
                    /// both valid for the call. Must be called after
                    /// `afterglow_wasm_init`. Single-threaded (one worker = one
                    /// instance on one thread).
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn afterglow_wasm_serve_frame(
                        method: u32,
                        args_ptr: *const u8,
                        args_len: usize,
                        out_ptr: *mut u8,
                        out_max: usize,
                    ) -> i32 {
                        // SAFETY: caller upholds pointer validity (see # Safety).
                        let args = unsafe { std::slice::from_raw_parts(args_ptr, args_len) };
                        let out = unsafe { std::slice::from_raw_parts_mut(out_ptr, out_max) };
                        // Always postcard-encode the `Response` envelope — success,
                        // server/decode errors, and unit/zero-byte results alike.
                        let env = WORKER.with(|w| match w.borrow_mut().as_deref_mut() {
                            Some(s) => ::afterglow_rpc::make_response(method, s.serve(method, args)),
                            None => ::afterglow_rpc::Response::Server {
                                method,
                                message: "wasm worker not initialized".into(),
                            },
                        });
                        // An oversized response is replaced with a tiny error envelope
                        // (if even that can't fit `out`, e.g. out_max == 0, bail with -1).
                        let bytes = match ::afterglow_rpc::encode(&env) {
                            Ok(b) if b.len() <= out.len() => b,
                            _ => match ::afterglow_rpc::encode(&::afterglow_rpc::Response::Server {
                                method,
                                message: "response too large".into(),
                            }) {
                                Ok(b) => b,
                                Err(_) => return -1,
                            },
                        };
                        let n = bytes.len();
                        if n > out.len() { return -1; } // even the error envelope may not fit
                        out[..n].copy_from_slice(&bytes);
                        n as i32
                    }

                    #[unsafe(no_mangle)]
                    pub extern "C" fn afterglow_wasm_input_ptr() -> usize {
                        INPUT.0.get() as *const u8 as usize
                    }
                    #[unsafe(no_mangle)]
                    pub extern "C" fn afterglow_wasm_input_size() -> usize { IN_SIZE }
                    #[unsafe(no_mangle)]
                    pub extern "C" fn afterglow_wasm_output_ptr() -> usize {
                        OUTPUT.0.get() as *const u8 as usize
                    }
                    #[unsafe(no_mangle)]
                    pub extern "C" fn afterglow_wasm_output_size() -> usize { OUT_SIZE }
                }
            };
            (spawn, wasm)
        }
        None => (quote! {}, quote! {}),
    };

    let expanded = quote! {
        #vis trait #server {
            #( #sigs )*
            /// Dispatch a `(method, args)` request to the matching trait method,
            /// returning the postcard-encoded result or an `RpcError`.
            fn serve(&mut self, method: u32, args: &[u8]) -> ::afterglow_rpc::RpcResult<Vec<u8>> {
                match method {
                    #( #arms )*
                    _ => Err(::afterglow_rpc::RpcError::UnknownMethod),
                }
            }
        }

        #vis struct #client<T: ::afterglow_rpc::Transport> { t: T }
        impl<T: ::afterglow_rpc::Transport> #client<T> {
            pub fn new(t: T) -> Self { Self { t } }
            /// Read-only access to the underlying transport (for ad-hoc/raw calls).
            pub fn transport(&self) -> &T { &self.t }
            #( #clients )*
        }

        #vis static #schema: &::afterglow_rpc::RpcSchema = &::afterglow_rpc::RpcSchema {
            name: #name_str,
            methods: &[#( #methods ),*],
        };

        #spawn
        #wasm
    };
    expanded.into()
}

/// Reject unsupported trait constructs with compile errors (never panic).
///
/// `worker` is the parsed `#[rpc(worker = Type)]` attribute (if any). It
/// extends the reserved method-name set with `spawn_worker`, which worker
/// mode generates as the native client constructor.
fn validate_trait(tr: &ItemTrait, worker: Option<&Type>) -> syn::Result<()> {
    use syn::TraitItem;
    macro_rules! bad {
        ($t:expr, $m:expr) => {
            return Err(syn::Error::new_spanned($t, $m))
        };
    }
    if !tr.generics.params.is_empty() || tr.generics.where_clause.is_some() {
        bad!(&tr.generics, "#[rpc] traits must not be generic");
    }
    if !tr.supertraits.is_empty() {
        bad!(&tr.supertraits, "#[rpc] traits must not have supertraits");
    }
    for item in &tr.items {
        let TraitItem::Fn(m) = item else {
            bad!(item, "#[rpc] traits may only contain methods");
        };
        let sig = &m.sig;
        // Method names that collide with the generated API surface. `serve`,
        // `new`, and `transport` are always generated; `spawn_worker` is only
        // generated in worker mode.
        let mname_str = sig.ident.to_string();
        let collides = matches!(mname_str.as_str(), "serve" | "new" | "transport")
            || (mname_str == "spawn_worker" && worker.is_some());
        if collides {
            bad!(
                &sig.ident,
                format!(
                    "#[rpc] method `{mname_str}` collides with a generated API name; rename it"
                )
            );
        }
        if m.default.is_some() {
            bad!(m, "#[rpc] methods must not have default bodies");
        }
        for (set, label) in [
            (sig.asyncness.is_some(), "async"),
            (sig.constness.is_some(), "const"),
            (sig.unsafety.is_some(), "unsafe"),
            (sig.abi.is_some(), "extern"),
        ] {
            if set {
                bad!(&sig.fn_token, format!("#[rpc] methods must not be {label}"));
            }
        }
        if let Some(v) = &sig.variadic {
            bad!(v, "#[rpc] methods must not be variadic");
        }
        if !sig.generics.params.is_empty() || sig.generics.where_clause.is_some() {
            bad!(&sig.generics, "#[rpc] methods must not be generic");
        }
        for a in &sig.inputs {
            match a {
                FnArg::Receiver(_) => {
                    bad!(a, "#[rpc] methods must not have a `self` receiver");
                }
                FnArg::Typed(pt) if !matches!(&*pt.pat, Pat::Ident(_)) => {
                    bad!(pt, "#[rpc] params must be simple identifiers");
                }
                FnArg::Typed(_) => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr(s: &str) -> syn::Result<RpcAttr> {
        syn::parse_str(s)
    }
    fn parsed(s: &str) -> ItemTrait {
        syn::parse_str(s).unwrap()
    }

    #[test]
    fn attr_empty_and_worker() {
        assert!(attr("").unwrap().worker.is_none());
        assert!(attr("worker = PhysicsWorker").unwrap().worker.is_some());
    }

    #[test]
    fn attr_rejects_invalid() {
        assert!(attr("foo").is_err());
        assert!(attr("worker").is_err());
        assert!(attr("worker = A extra").is_err());
        assert!(attr("worker = A, x").is_err());
    }

    /// A dummy worker type — validation only checks `is_some()`.
    fn worker_type() -> Type {
        syn::parse_str("PhysicsWorker").unwrap()
    }

    #[test]
    fn valid_trait_ok() {
        assert!(
            validate_trait(
                &parsed("pub trait Foo { fn add(a: u32, b: u32) -> u32; }"),
                None
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_unsupported_shapes() {
        assert!(validate_trait(&parsed("trait T<X> { fn a(); }"), None).is_err()); // generic
        assert!(validate_trait(&parsed("trait T: Sized { fn a(); }"), None).is_err()); // supertrait
        assert!(validate_trait(&parsed("trait T { fn a(&self); }"), None).is_err()); // receiver
        assert!(validate_trait(&parsed("trait T { async fn a(); }"), None).is_err()); // async
        assert!(validate_trait(&parsed("trait T { const fn a(); }"), None).is_err()); // const
        assert!(
            validate_trait(&parsed(r#"trait T { unsafe extern "C" fn a(...); }"#), None).is_err()
        );
        assert!(validate_trait(&parsed("trait T { const C: u32; }"), None).is_err()); // assoc item
        assert!(validate_trait(&parsed("trait T { fn a() {} }"), None).is_err()); // default body
        assert!(validate_trait(&parsed("trait T { fn a((x,): (u32,)); }"), None).is_err()); // non-ident
        assert!(validate_trait(&parsed("trait T { fn a<X>(x: u32); }"), None).is_err()); // generic method
        assert!(validate_trait(&parsed("trait T { fn a() where u32: Copy; }"), None).is_err()); // where clause
    }

    #[test]
    fn rejects_reserved_method_names() {
        // Always reserved: `serve` (server dispatch) and `new`/`transport`
        // (generic client methods). Rejected regardless of worker mode.
        for name in ["serve", "new", "transport"] {
            let src = format!("trait T {{ fn {name}(a: u32) -> u32; }}");
            assert!(
                validate_trait(&parsed(&src), None).is_err(),
                "`{name}` should be reserved without worker"
            );
            assert!(
                validate_trait(&parsed(&src), Some(&worker_type())).is_err(),
                "`{name}` should be reserved with worker"
            );
        }
    }

    #[test]
    fn spawn_worker_reserved_only_in_worker_mode() {
        let src = "trait T { fn spawn_worker(a: u32) -> u32; }";
        // Without worker: allowed (spawn_worker is not generated).
        assert!(validate_trait(&parsed(src), None).is_ok());
        // With worker: collides with the generated native constructor.
        assert!(validate_trait(&parsed(src), Some(&worker_type())).is_err());
    }

    #[test]
    fn normal_method_names_validate_with_and_without_worker() {
        let src = "trait Physics { fn step(state: Vec<f32>, dt: f32) -> Vec<f32>; }";
        assert!(validate_trait(&parsed(src), None).is_ok());
        assert!(validate_trait(&parsed(src), Some(&worker_type())).is_ok());
    }
}
