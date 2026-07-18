//! `#[rpc]` on a trait generates:
//! - `<Name>Server` — the trait a worker implements, with a **provided**
//!   `serve(&mut self, method, args) -> RpcResult<Vec<u8>>` dispatch (no free
//!   `serve` fn, so two traits in one module don't collide).
//! - `<Name>Client<T: Transport>` — a typed client with per-method wrappers
//!   and a `transport(&self) -> &T` accessor (fields stay private).
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
use quote::{format_ident, quote};
use syn::{FnArg, Ident, ItemTrait, Pat, TraitItem, Type};

mod ts;

/// Parsed `#[rpc]` attribute: `worker = <Type>` (optionally `singleton`).
struct RpcAttr {
    worker: Option<Type>,
    singleton: bool,
}

impl syn::parse::Parse for RpcAttr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self {
                worker: None,
                singleton: false,
            });
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
        let mut singleton = false;
        // Optional `, singleton`.
        while input.peek(syn::Token![,]) {
            let _: syn::Token![,] = input.parse()?;
            let flag: Ident = input.parse()?;
            match flag.to_string().as_str() {
                "singleton" => singleton = true,
                other => {
                    return Err(syn::Error::new(
                        flag.span(),
                        format!("unsupported #[rpc] flag `{other}`; expected `singleton`"),
                    ));
                }
            }
        }
        if !input.is_empty() {
            return Err(syn::Error::new(
                input.span(),
                "unexpected tokens after `worker = Type`",
            ));
        }
        Ok(Self {
            worker: Some(ty),
            singleton,
        })
    }
}

#[proc_macro_attribute]
pub fn rpc(attr: TokenStream, item: TokenStream) -> TokenStream {
    let parsed_attr = match syn::parse::<RpcAttr>(attr) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error().into(),
    };
    let worker = parsed_attr.worker;
    let singleton = parsed_attr.singleton;
    let tr = syn::parse_macro_input!(item as ItemTrait);
    if let Err(e) = validate_trait(&tr, worker.as_ref()) {
        return e.to_compile_error().into();
    }

    // Detect async: if any method is `async fn`, the whole trait is async (poll model).
    let is_async = tr
        .items
        .iter()
        .any(|item| matches!(item, TraitItem::Fn(m) if m.sig.asyncness.is_some()));

    // Generate the typed TS client when `worker = Type` (wasm exports exist →
    // JS needs a client). Non-worker traits are Rust-only; no TS needed.
    if worker.is_some() {
        if let Err(e) = ts::generate_and_write(&tr, is_async) {
            return e.to_compile_error().into();
        }
    }

    let vis = &tr.vis;
    let name = &tr.ident;
    let server = format_ident!("{}Server", name);
    let client = format_ident!("{}Client", name);
    let mut sigs = Vec::new();
    let mut clients = Vec::new();
    let mut arms = Vec::new();

    for (id, item) in tr.items.iter().enumerate() {
        let TraitItem::Fn(m) = item else { continue }; // validate_trait rejected the rest
        let sig = &m.sig;
        let mname = &sig.ident;
        let id_lit = id as u32;

        let mut pats = Vec::new();
        let mut tys = Vec::new();
        for a in &sig.inputs {
            let FnArg::Typed(pt) = a else { continue }; // receivers rejected
            let Pat::Ident(pi) = &*pt.pat else { continue }; // idents only
            let pat = &pi.ident;
            let ty = &pt.ty;
            pats.push(quote! { #pat });
            tys.push(quote! { #ty });
        }
        let ret: Type = match &sig.output {
            syn::ReturnType::Type(_, t) => (**t).clone(),
            _ => syn::parse_quote!(()),
        };
        // Trailing commas force tuple semantics for the 1-param case (`(a,)`),
        // so single-arg methods round-trip like multi-arg ones.
        if is_async {
            // Async: server trait returns ServeFuture, client returns a Future.
            sigs.push(
                quote! { fn #mname(&self, #( #pats: #tys ),*) -> ::afterglow_rpc::ServeFuture; },
            );
            clients.push(quote! {
                pub fn #mname(&self, #( #pats: #tys ),*)
                    -> ::afterglow_rpc::RpcResult<impl ::std::future::Future<Output = #ret>>
                {
                    let args = ::afterglow_rpc::encode(&(#( #pats, )*))?;
                    let rx = self.transport.call_async(#id_lit, &args)?;
                    Ok(async move {
                        let bytes = rx.await?;
                        ::afterglow_rpc::decode(&bytes)
                    })
                }
            });
            arms.push(quote! {
                #id_lit => {
                    match ::afterglow_rpc::decode::<(#( #tys, )*)>(args) {
                        Ok((#( #pats, )*)) => self.#mname(#( #pats ),*),
                        Err(e) => ::std::boxed::Box::pin(async move { Err(e) }),
                    }
                }
            });
        } else {
            sigs.push(quote! { fn #mname(&mut self, #( #pats: #tys ),*) -> #ret; });
            clients.push(quote! {
                pub fn #mname(&self, #( #pats: #tys ),*) -> ::afterglow_rpc::RpcResult<#ret> {
                    let resp = self.t.call(#id_lit, &::afterglow_rpc::encode(&(#( #pats, )*))?)?;
                    ::afterglow_rpc::decode(&resp)
                }
            });
            arms.push(quote! {
                #id_lit => {
                    let (#( #pats, )*): (#( #tys, )*) = ::afterglow_rpc::decode(args)?;
                    ::afterglow_rpc::encode(&self.#mname(#( #pats ),*))
                }
            });
        }
    }

    let (spawn, wasm, client_struct) = if is_async {
        // Async: native uses AsyncWorkerTransport + spawn_async_worker_loop.
        // Wasm: exports for the poll model — serve_async (spawn task, return
        // task_id), tick (drive executor), drain_completion (pop a completion).
        if let Some(wt) = &worker {
            let (spawn, client_struct) = if singleton {
                // Singleton: the transport is Arc'd, the client is Clone, and
                // spawn_worker uses a OnceLock so only one worker thread is
                // ever created. Subsequent calls clone the Arc.
                let spawn = quote! {
                    #[cfg(not(target_arch = "wasm32"))]
                    static ASYNC_WORKER_SINGLETON: ::std::sync::Mutex<
                        ::std::option::Option<::std::sync::Weak<::afterglow_rpc::native::AsyncWorkerTransport>>
                    > = ::std::sync::Mutex::new(::std::option::Option::None);

                    #[cfg(not(target_arch = "wasm32"))]
                    impl #client {
                        /// Spawn the singleton worker. The first call creates the
                        /// worker (via `#wt::default()`); subsequent calls return a
                        /// clone sharing the same worker. If all clients have been
                        /// dropped, a new call re-spawns. Thread-safe.
                        pub fn spawn_worker()
                            -> ::afterglow_rpc::RpcResult<Self>
                        {
                            let mut guard = ASYNC_WORKER_SINGLETON.lock().unwrap();
                            // Try to upgrade an existing weak ref — if it succeeds,
                            // the worker is still alive.
                            if let ::std::option::Option::Some(weak) = guard.as_ref() {
                                if let ::std::option::Option::Some(strong) = weak.upgrade() {
                                    return Ok(Self { transport: strong });
                                }
                            }
                            // No worker yet, or all clients dropped (weak is stale).
                            let impl_ = #wt::default();
                            let serve = |s: &#wt, m: u32, a: &[u8]| s.serve_async(m, a);
                            let (t, _ev) = ::afterglow_rpc::native::spawn_async_worker_loop(impl_, 1 << 20, serve)?;
                            let strong = ::std::sync::Arc::new(t);
                            *guard = ::std::option::Option::Some(::std::sync::Arc::downgrade(&strong));
                            Ok(Self { transport: strong })
                        }
                    }
                };
                (
                    spawn,
                    quote! {
                        #[cfg(not(target_arch = "wasm32"))]
                        #vis struct #client {
                            transport: ::std::sync::Arc<::afterglow_rpc::native::AsyncWorkerTransport>,
                        }

                        #[cfg(not(target_arch = "wasm32"))]
                        impl ::std::clone::Clone for #client {
                            fn clone(&self) -> Self {
                                Self { transport: ::std::sync::Arc::clone(&self.transport) }
                            }
                        }

                        #[cfg(not(target_arch = "wasm32"))]
                        impl #client {
                            pub fn new(t: ::std::sync::Arc<::afterglow_rpc::native::AsyncWorkerTransport>) -> Self {
                                Self { transport: t }
                            }
                        }
                    },
                )
            } else {
                // Per-spawn: each call creates a new worker thread.
                let spawn = quote! {
                    #[cfg(not(target_arch = "wasm32"))]
                    impl #client {
                        /// Spawn the async worker as a native OS thread with an
                        /// `async-executor::LocalExecutor`. Returns the typed client
                        /// (with `poll()`) + event receiver. The worker thread is
                        /// joined when the client is dropped.
                        pub fn spawn_worker(impl_: #wt)
                            -> ::afterglow_rpc::RpcResult<(Self, ::afterglow_rpc::native::EventReceiver)>
                        {
                            let serve = |s: &#wt, m: u32, a: &[u8]| s.serve_async(m, a);
                            let (t, ev) = ::afterglow_rpc::native::spawn_async_worker_loop(impl_, 1 << 20, serve)?;
                            Ok((Self { transport: t }, ev))
                        }
                    }
                };
                (
                    spawn,
                    quote! {
                        #[cfg(not(target_arch = "wasm32"))]
                        #vis struct #client { transport: ::afterglow_rpc::native::AsyncWorkerTransport }

                        #[cfg(not(target_arch = "wasm32"))]
                        impl #client {
                            pub fn new(t: ::afterglow_rpc::native::AsyncWorkerTransport) -> Self { Self { transport: t } }
                        }
                    },
                )
            };
            let wasm = quote! {
                // Wasm async worker exports (poll model). `worker.js` drives these:
                //   1. afterglow_wasm_init() — construct the worker + executor.
                //   2. afterglow_wasm_serve_async(method, args, task_id) — spawn task.
                //   3. afterglow_wasm_tick() — drive the executor (re-polls pending tasks).
                //   4. afterglow_wasm_drain_completion(out, max) -> i32 — pop a completion.
                #[cfg(target_arch = "wasm32")]
                mod afterglow_wasm {
                    use super::{#server, #wt};
                    use std::cell::RefCell;
                    use ::afterglow_rpc::ServeFuture;
                    use ::afterglow_rpc::wasm::Scratch;

                    const IN_SIZE: usize = 1 << 20;
                    const OUT_SIZE: usize = 1 << 20;
                    const COMPLETION_CAPACITY: usize = 256;

                    static INPUT: Scratch<{ IN_SIZE }> = Scratch::new();
                    static OUTPUT: Scratch<{ OUT_SIZE }> = Scratch::new();

                    thread_local! {
                        static WORKER: RefCell<Option<#wt>> = const { RefCell::new(None) };
                        static EXECUTOR: RefCell<Option<async_executor::LocalExecutor<'static>>> = const { RefCell::new(None) };
                        static COMPLETIONS: RefCell<Option<std::collections::VecDeque<Vec<u8>>>> = const { RefCell::new(None) };
                        static OUTSTANDING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
                    }

                    /// Construct the worker impl + executor. JS calls this once.
                    /// Requires `#wt: Default`.
                    #[unsafe(no_mangle)]
                    pub extern "C" fn afterglow_wasm_init() {
                        WORKER.with(|w| *w.borrow_mut() = Some(#wt::default()));
                        EXECUTOR.with(|e| *e.borrow_mut() = Some(async_executor::LocalExecutor::new()));
                        COMPLETIONS.with(|c| *c.borrow_mut() = Some(std::collections::VecDeque::with_capacity(COMPLETION_CAPACITY)));
                        OUTSTANDING.with(|count| count.set(0));
                    }

                    /// Spawn an async task for `(method, args)` with the given `task_id`.
                    /// Returns 0 on success, -1 if the worker isn't initialized.
                    /// Does NOT block — the task runs when `afterglow_wasm_tick` is called.
                    #[unsafe(no_mangle)]
                    pub extern "C" fn afterglow_wasm_serve_async(
                        method: u32,
                        args_ptr: *const u8,
                        args_len: usize,
                        task_id: u64,
                    ) -> i32 {
                        let admitted = OUTSTANDING.with(|count| {
                            if count.get() >= COMPLETION_CAPACITY { false }
                            else { count.set(count.get() + 1); true }
                        });
                        if !admitted { return -2; }
                        // SAFETY: caller upholds pointer validity.
                        let args = unsafe { std::slice::from_raw_parts(args_ptr, args_len) };
                        let args_owned = args.to_vec();
                        let worker_ok = WORKER.with(|w| {
                            let wcell = w.borrow();
                            let Some(worker) = wcell.as_ref() else { return false; };
                            let fut: ::afterglow_rpc::ServeFuture = worker.serve_async(method, &args_owned);
                            EXECUTOR.with(|e| {
                                if let Some(exec) = e.borrow_mut().as_mut() {
                                    let task = exec.spawn(async move {
                                        let result = fut.await;
                                        let env = ::afterglow_rpc::make_response(method, result);
                                        let env_bytes = ::afterglow_rpc::encode(&env).unwrap_or_default();
                                        let mut completion = Vec::with_capacity(8 + env_bytes.len());
                                        completion.extend_from_slice(&task_id.to_le_bytes());
                                        completion.extend_from_slice(&env_bytes);
                                        COMPLETIONS.with(|c| {
                                            let mut cell = c.borrow_mut();
                                            let queue = cell.as_mut().expect("async worker completion queue not initialized");
                                            // JS admits at most COMPLETION_CAPACITY task slots, so
                                            // every in-flight task has one fixed completion slot.
                                            assert!(queue.len() < COMPLETION_CAPACITY, "async worker completion capacity exceeded");
                                            queue.push_back(completion);
                                        });
                                    });
                                    task.detach();
                                }
                            });
                            true
                        });
                        if worker_ok { 0 } else {
                            OUTSTANDING.with(|count| count.set(count.get().saturating_sub(1)));
                            -1
                        }
                    }

                    /// Drive the executor: poll pending tasks once. Returns the
                    /// number of completions now available to drain.
                    #[unsafe(no_mangle)]
                    pub extern "C" fn afterglow_wasm_tick() -> i32 {
                        EXECUTOR.with(|e| {
                            if let Some(exec) = e.borrow_mut().as_mut() {
                                exec.try_tick();
                            }
                        });
                        COMPLETIONS.with(|c| c.borrow().as_ref().map_or(0, |queue| queue.len() as i32))
                    }

                    /// Pop one completion `[task_id][Response]` into `out`.
                    /// Returns the byte length, or -1 if none available.
                    ///
                    /// # Safety
                    /// `out` must point to `out_max` writable bytes.
                    #[unsafe(no_mangle)]
                    pub unsafe extern "C" fn afterglow_wasm_drain_completion(
                        out_ptr: *mut u8,
                        out_max: usize,
                    ) -> i32 {
                        // SAFETY: caller upholds pointer validity.
                        let out = unsafe { std::slice::from_raw_parts_mut(out_ptr, out_max) };
                        COMPLETIONS.with(|c| {
                            let mut cell = c.borrow_mut();
                            let Some(queue) = cell.as_mut() else { return -1; };
                            match queue.pop_front() {
                                Some(comp) if comp.len() <= out.len() => {
                                    out[..comp.len()].copy_from_slice(&comp);
                                    OUTSTANDING.with(|count| count.set(count.get().saturating_sub(1)));
                                    comp.len() as i32
                                }
                                Some(comp) => {
                                    // Doesn't fit — push it back. Caller retries with a larger buffer.
                                    queue.push_front(comp);
                                    -2
                                }
                                None => -1,
                            }
                        })
                    }

                    #[unsafe(no_mangle)]
                    pub extern "C" fn afterglow_wasm_input_ptr() -> usize { INPUT.ptr() }
                    #[unsafe(no_mangle)]
                    pub extern "C" fn afterglow_wasm_input_size() -> usize { INPUT.size() }
                    #[unsafe(no_mangle)]
                    pub extern "C" fn afterglow_wasm_output_ptr() -> usize { OUTPUT.ptr() }
                    #[unsafe(no_mangle)]
                    pub extern "C" fn afterglow_wasm_output_size() -> usize { OUTPUT.size() }
                }
            };
            (spawn, wasm, client_struct)
        } else {
            (quote! {}, quote! {}, quote! {})
        }
    } else {
        // Sync: existing path (spawn_worker_loop + wasm exports).
        match &worker {
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
                        use std::cell::RefCell;
                        use ::afterglow_rpc::wasm::Scratch;

                        const IN_SIZE: usize = 1 << 20;
                        const OUT_SIZE: usize = 1 << 20;

                        static INPUT: Scratch<{ IN_SIZE }> = Scratch::new();
                        static OUTPUT: Scratch<{ OUT_SIZE }> = Scratch::new();

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
                            let result = WORKER.with(|w| match w.borrow_mut().as_deref_mut() {
                                Some(s) => s.serve(method, args),
                                None => Err(::afterglow_rpc::RpcError::Server(
                                    "wasm worker not initialized".into(),
                                )),
                            });
                            ::afterglow_rpc::wasm::write_response(method, result, out)
                        }

                        #[unsafe(no_mangle)]
                        pub extern "C" fn afterglow_wasm_input_ptr() -> usize {
                            INPUT.ptr()
                        }
                        #[unsafe(no_mangle)]
                        pub extern "C" fn afterglow_wasm_input_size() -> usize { INPUT.size() }
                        #[unsafe(no_mangle)]
                        pub extern "C" fn afterglow_wasm_output_ptr() -> usize {
                            OUTPUT.ptr()
                        }
                        #[unsafe(no_mangle)]
                        pub extern "C" fn afterglow_wasm_output_size() -> usize { OUTPUT.size() }
                    }
                };
                (spawn, wasm, quote! {})
            }
            None => (quote! {}, quote! {}, quote! {}),
        }
    };

    let expanded = if is_async {
        quote! {
            #vis trait #server {
                #( #sigs )*
                /// Dispatch a `(method, args)` request to the matching async trait
                /// method, returning a [`ServeFuture`] that resolves to the
                /// postcard-encoded result or an `RpcError`.
                fn serve_async(&self, method: u32, args: &[u8]) -> ::afterglow_rpc::ServeFuture {
                    match method {
                        #( #arms )*
                        _ => ::std::boxed::Box::pin(async { Err(::afterglow_rpc::RpcError::UnknownMethod) }),
                    }
                }
            }

            #client_struct

            #[cfg(not(target_arch = "wasm32"))]
            impl #client {
                /// Drain completions and resolve pending futures. Call each frame.
                pub fn poll(&self) { self.transport.poll(); }
                #( #clients )*
            }

            #spawn
            #wasm
        }
    } else {
        quote! {
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

            #spawn
            #wasm
        }
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
        // async is now ALLOWED (async #[rpc] with poll model).
        assert!(validate_trait(&parsed("trait T { async fn a(); }"), None).is_ok());
        assert!(validate_trait(&parsed("trait T { const fn a(); }"), None).is_err()); // const
        assert!(
            validate_trait(&parsed(r#"trait T { unsafe extern "C" fn a(...); }"#), None).is_err()
        );
        // async is now ALLOWED (async #[rpc] with poll model).
        assert!(validate_trait(&parsed("trait T { async fn a(); }"), None).is_ok());
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

    // --- TS client generation tests ---

    #[test]
    fn ts_generates_physics_client() {
        let tr = parsed(
            "pub trait Physics { fn step(state: Vec<f32>, dt: f32) -> Vec<f32>; fn apply_force(body_id: u32, fx: f32, fy: f32, fz: f32) -> bool; }",
        );
        let ts = crate::ts::generate_client(&tr, false).unwrap();
        assert!(ts.contains("export class PhysicsClient"), "{ts}");
        assert!(
            ts.contains("async step(state: Float32Array, dt: number): Promise<Float32Array>"),
            "{ts}"
        );
        assert!(ts.contains("async applyForce(bodyId: number, fx: number, fy: number, fz: number): Promise<boolean>"), "{ts}");
        assert!(ts.contains("this.rpc.call(0,"), "method 0 id");
        assert!(ts.contains("this.rpc.call(1,"), "method 1 id");
        // imports used
        assert!(ts.contains("encodeF32Vec"), "{ts}");
        assert!(ts.contains("encodeU32"), "{ts}");
        assert!(ts.contains("decodeF32Vec"), "{ts}");
        assert!(ts.contains("decodeBool"), "{ts}");
        assert!(ts.contains("concat"), "{ts}");
        assert!(ts.contains("close(): void { if (this.closed) return; this.closed = true; this.rpc.terminate?.(); }"), "{ts}");
    }

    #[test]
    fn ts_handles_void_return() {
        let tr = parsed("pub trait Logger { fn log(msg: String); }");
        let ts = crate::ts::generate_client(&tr, false).unwrap();
        assert!(ts.contains("async log(msg: string): Promise<void>"), "{ts}");
        assert!(ts.contains("await this.rpc.call(0, args)"), "{ts}");
        // The void method body should not return a decoded value — check the
        // method body specifically (spawn() has return statements, that's fine).
        assert!(
            !ts.contains("return decode"),
            "void method must not return a decoded value"
        );
    }

    #[test]
    fn ts_rejects_unsupported_type() {
        let tr = parsed("pub trait Bad { fn load(p: String) -> HashMap<String, u32>; }");
        let err = crate::ts::generate_client(&tr, false);
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(msg.contains("HashMap"), "{msg}");
    }

    #[test]
    fn ts_camel_case_conversion() {
        let tr = parsed("pub trait S { fn do_thing(a: u32) -> u32; fn a_b_c(x: u32) -> u32; }");
        let ts = crate::ts::generate_client(&tr, false).unwrap();
        assert!(ts.contains("async doThing("), "{ts}");
        assert!(ts.contains("async aBC("), "{ts}");
    }

    // --- TS type matrix: every supported type maps correctly ---

    #[test]
    fn ts_type_matrix_all_primitives() {
        // Every supported primitive type as a param + return.
        let tr = parsed(
            "pub trait Types { fn float32(x: f32) -> f32; fn float64(x: f64) -> f64; fn byte(x: u8) -> u8; fn word(x: u16) -> u16; fn dword(x: u32) -> u32; fn qword(x: u64) -> u64; fn count(x: usize) -> usize; fn sbyte(x: i8) -> i8; fn sword(x: i16) -> i16; fn sdword(x: i32) -> i32; fn sqword(x: i64) -> i64; fn offset(x: isize) -> isize; fn flag(x: bool) -> bool; }",
        );
        let ts = crate::ts::generate_client(&tr, false).unwrap();
        // f32/f64 → number
        assert!(
            ts.contains("async float32(x: number): Promise<number>"),
            "{ts}"
        );
        assert!(
            ts.contains("async float64(x: number): Promise<number>"),
            "{ts}"
        );
        // unsigned ints → number
        assert!(
            ts.contains("async byte(x: number): Promise<number>"),
            "{ts}"
        );
        assert!(
            ts.contains("async word(x: number): Promise<number>"),
            "{ts}"
        );
        assert!(
            ts.contains("async dword(x: number): Promise<number>"),
            "{ts}"
        );
        assert!(
            ts.contains("async qword(x: number): Promise<number>"),
            "{ts}"
        );
        assert!(
            ts.contains("async count(x: number): Promise<number>"),
            "{ts}"
        );
        // signed ints → number
        assert!(
            ts.contains("async sbyte(x: number): Promise<number>"),
            "{ts}"
        );
        assert!(
            ts.contains("async sword(x: number): Promise<number>"),
            "{ts}"
        );
        assert!(
            ts.contains("async sdword(x: number): Promise<number>"),
            "{ts}"
        );
        assert!(
            ts.contains("async sqword(x: number): Promise<number>"),
            "{ts}"
        );
        assert!(
            ts.contains("async offset(x: number): Promise<number>"),
            "{ts}"
        );
        // bool → boolean
        assert!(
            ts.contains("async flag(x: boolean): Promise<boolean>"),
            "{ts}"
        );
        // Correct codec functions are imported.
        assert!(ts.contains("encodeF32"), "{ts}");
        assert!(ts.contains("encodeF64"), "{ts}");
        assert!(ts.contains("encodeU8"), "{ts}");
        assert!(ts.contains("encodeU16"), "{ts}");
        assert!(ts.contains("encodeU32"), "{ts}");
        assert!(ts.contains("encodeU64"), "{ts}");
        assert!(ts.contains("encodeI8"), "{ts}");
        assert!(ts.contains("encodeI16"), "{ts}");
        assert!(ts.contains("encodeI32"), "{ts}");
        assert!(ts.contains("encodeI64"), "{ts}");
        assert!(ts.contains("encodeBool"), "{ts}");
        assert!(ts.contains("decodeF32"), "{ts}");
        assert!(ts.contains("decodeF64"), "{ts}");
        assert!(ts.contains("decodeU8"), "{ts}");
        assert!(ts.contains("decodeU16"), "{ts}");
        assert!(ts.contains("decodeU32"), "{ts}");
        assert!(ts.contains("decodeU64"), "{ts}");
        assert!(ts.contains("decodeI8"), "{ts}");
        assert!(ts.contains("decodeI16"), "{ts}");
        assert!(ts.contains("decodeI32"), "{ts}");
        assert!(ts.contains("decodeI64"), "{ts}");
        assert!(ts.contains("decodeBool"), "{ts}");
    }

    #[test]
    fn ts_type_matrix_strings_and_vectors() {
        let tr = parsed(
            "pub trait Coll { fn name(s: String) -> String; fn raw(b: Vec<u8>) -> Vec<u8>; fn f32s(v: Vec<f32>) -> Vec<f32>; fn f64s(v: Vec<f64>) -> Vec<f64>; }",
        );
        let ts = crate::ts::generate_client(&tr, false).unwrap();
        assert!(
            ts.contains("async name(s: string): Promise<string>"),
            "{ts}"
        );
        assert!(
            ts.contains("async raw(b: Uint8Array): Promise<Uint8Array>"),
            "{ts}"
        );
        assert!(
            ts.contains("async f32s(v: Float32Array): Promise<Float32Array>"),
            "{ts}"
        );
        assert!(
            ts.contains("async f64s(v: Float64Array): Promise<Float64Array>"),
            "{ts}"
        );
        assert!(ts.contains("encodeString"), "{ts}");
        assert!(ts.contains("encodeBytes"), "{ts}");
        assert!(ts.contains("encodeF32Vec"), "{ts}");
        assert!(ts.contains("encodeF64Vec"), "{ts}");
        assert!(ts.contains("decodeString"), "{ts}");
        assert!(ts.contains("decodeBytes"), "{ts}");
        assert!(ts.contains("decodeF32Vec"), "{ts}");
        assert!(ts.contains("decodeF64Vec"), "{ts}");
    }

    #[test]
    fn ts_rpc_result_return_maps_to_inner_type() {
        // RpcResult<T> → T in TS (errors are thrown).
        let tr = parsed("pub trait R { fn load(p: String) -> RpcResult<Vec<u8>>; }");
        let ts = crate::ts::generate_client(&tr, false).unwrap();
        assert!(
            ts.contains("async load(p: string): Promise<Uint8Array>"),
            "{ts}"
        );
        assert!(ts.contains("decodeBytes"), "{ts}");
        // No mention of RpcResult in the TS type.
        assert!(!ts.contains("RpcResult"), "{ts}");
    }

    #[test]
    fn ts_multi_param_mixed_types() {
        let tr = parsed(
            "pub trait Mixed { fn go(name: String, count: u32, active: bool, data: Vec<f32>) -> u64; }",
        );
        let ts = crate::ts::generate_client(&tr, false).unwrap();
        assert!(ts.contains("async go(name: string, count: number, active: boolean, data: Float32Array): Promise<number>"), "{ts}");
        // All encode fns are imported + used in concat().
        assert!(ts.contains("concat(encodeString(name), encodeU32(count), encodeBool(active), encodeF32Vec(data))"), "{ts}");
    }

    #[test]
    fn ts_no_params_void_return() {
        let tr = parsed("pub trait N { fn ping(); }");
        let ts = crate::ts::generate_client(&tr, false).unwrap();
        assert!(ts.contains("async ping(): Promise<void>"), "{ts}");
        assert!(ts.contains("new Uint8Array()"), "{ts}");
        assert!(ts.contains("await this.rpc.call(0, args)"), "{ts}");
    }
}
