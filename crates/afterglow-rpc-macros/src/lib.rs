//! `#[rpc]` on a trait generates:
//! - `<Name>Server` — the trait a worker implements.
//! - `<Name>Client<T: Transport>` — a client with typed methods (talks to the
//!   worker as if it were local).
//! - `serve(server, method_id, args)` — server-side dispatch.
//! - `SCHEMA: RpcSchema` — for the build system to generate TypeScript clients.

use proc_macro::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{parse_macro_input, FnArg, ItemTrait, ReturnType, TraitItem};

#[proc_macro_attribute]
pub fn rpc(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let tr = parse_macro_input!(item as ItemTrait);
    let name = &tr.ident;
    let server_name = format_ident!("{}Server", name);
    let client_name = format_ident!("{}Client", name);
    let name_str = name.to_string();

    let mut server_methods = Vec::new();
    let mut client_methods = Vec::new();
    let mut serve_arms = Vec::new();
    let mut schema_methods = Vec::new();

    for (id, item) in tr.items.iter().enumerate() {
        let TraitItem::Fn(m) = item else { continue };
        let sig = &m.sig;
        let mname = &sig.ident;
        let mname_str = mname.to_string();
        let id_lit = id as u32;

        // params (skip self)
        let mut pats = Vec::new();
        let mut tys = Vec::new();
        let mut param_pairs = Vec::new();
        for a in &sig.inputs {
            if let FnArg::Typed(pt) = a {
                let pat = &pt.pat;
                let ty = &pt.ty;
                let ty_str = ty.to_token_stream().to_string().replace(' ', "");
                let pat_str = pat.to_token_stream().to_string();
                pats.push(quote! { #pat });
                tys.push(quote! { #ty });
                param_pairs.push((pat_str, ty_str));
            }
        }
        let ret: syn::Type = match &sig.output {
            ReturnType::Type(_, t) => (**t).clone(),
            _ => syn::parse_quote!(()),
        };
        let ret_str = ret.to_token_stream().to_string().replace(' ', "");

        // server trait method
        server_methods.push(quote! {
            fn #mname(&mut self, #( #pats: #tys ),*) -> #ret;
        });

        // client method (sync; args moved into a tuple, postcard-encoded)
        client_methods.push(quote! {
            pub fn #mname(&self, #( #pats: #tys ),*) -> ::afterglow_rpc::RpcResult<#ret> {
                let args = (#( #pats ),*);
                let bytes = ::afterglow_rpc::encode(&args)?;
                let resp = self.t.call(#name_str, #id_lit, &bytes)?;
                ::afterglow_rpc::decode(&resp)
            }
        });

        // serve dispatch arm
        let tys_tuple = quote! { (#( #tys ),*) };
        serve_arms.push(quote! {
            #id_lit => {
                let (#( #pats ),*): #tys_tuple = ::afterglow_rpc::decode(args)?;
                let r = s.#mname(#( #pats ),*);
                ::afterglow_rpc::encode(&r)
            }
        });

        // schema entry
        let ppairs = param_pairs.iter().map(|(p, t)| quote! { (#p, #t) });
        schema_methods.push(quote! {
            ::afterglow_rpc::RpcMethod {
                id: #id_lit, name: #mname_str,
                params: &[#( #ppairs ),*],
                returns: #ret_str,
            }
        });
    }

    let expanded = quote! {
        /// Server trait: implement this on the worker side.
        pub trait #server_name {
            #( #server_methods )*
        }

        /// Client: construct with a Transport and call methods as if local.
        pub struct #client_name<T: ::afterglow_rpc::Transport> {
            t: T,
        }
        impl<T: ::afterglow_rpc::Transport> #client_name<T> {
            pub fn new(t: T) -> Self { Self { t } }
            #( #client_methods )*
        }

        /// Server-side dispatch: decode args, call the impl, encode the return.
        pub fn serve<S: #server_name + ?Sized>(s: &mut S, method: u32, args: &[u8]) -> ::afterglow_rpc::RpcResult<Vec<u8>> {
            match method {
                #( #serve_arms )*
                _ => Err(::afterglow_rpc::RpcError::UnknownMethod),
            }
        }

        /// Schema for build-system codegen (TypeScript + Rust client generation).
        #[allow(non_upper_case_globals)]
        pub static SCHEMA: ::afterglow_rpc::RpcSchema = ::afterglow_rpc::RpcSchema {
            name: #name_str,
            methods: &[#( #schema_methods ),*],
        };

        /// Spawn the worker as a native OS thread talking over a shared-memory
        /// ring buffer (zero-copy, no IPC per call). Returns a client (RPC) +
        /// an event receiver the main game context drains each frame.
        #[cfg(not(target_arch = "wasm32"))]
        pub fn spawn_worker<S: #server_name + Send + 'static>(impl_: S)
            -> (#client_name<::afterglow_rpc::native::WorkerTransport>, ::afterglow_rpc::native::EventReceiver)
        {
            use std::sync::mpsc::channel;
            let (event_tx, event_rx) = channel();
            let (transport, bufs) = ::afterglow_rpc::native::WorkerTransport::new_pair(1 << 20); // 1 MiB ring
            std::thread::spawn(move || {
                ::afterglow_rpc::native::run_worker_loop(impl_, bufs, event_tx, serve::<S>);
            });
            (#client_name::new(transport), ::afterglow_rpc::native::EventReceiver { rx: event_rx })
        }

        /// Web worker: initialize with a server impl. The JS worker calls
        /// `wasm_serve_frame` after this. The worker's wasm instance has its
        /// OWN memory (not shared with the main thread), so allocations
        /// (decode/encode inside `serve`) are safe — no allocator conflict.
        #[cfg(target_arch = "wasm32")]
        static mut WORKER: Option<Box<dyn #server_name>> = None;

        #[cfg(target_arch = "wasm32")]
        pub fn wasm_init_worker(impl_: Box<dyn #server_name>) {
            unsafe { WORKER = Some(impl_); }
        }

        /// Get a client that talks to the web worker over the SharedArrayBuffer
        /// ring buffer. Uses `afterglow_web::WebTransport` which encodes args
        /// (postcard), writes to the request ring buffer (notifying the worker
        /// via `AtomicU32::notify`), spins on the response, and decodes.
        #[cfg(target_arch = "wasm32")]
        pub fn get_client() -> #client_name<::afterglow_web::WebTransport> {
            #client_name::new(::afterglow_web::WebTransport)
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn wasm_serve_frame(method: u32, args_ptr: *const u8, args_len: usize, out_ptr: *mut u8, out_max_len: usize) -> i32 {
            let args = unsafe { std::slice::from_raw_parts(args_ptr, args_len) };
            let out = unsafe { std::slice::from_raw_parts_mut(out_ptr, out_max_len) };
            let impl_ = unsafe { WORKER.as_deref_mut().expect("wasm_init_worker not called") };
            match serve(impl_, method, args) {
                Ok(resp) => {
                    let n = resp.len().min(out.len());
                    out[..n].copy_from_slice(&resp[..n]);
                    n as i32
                }
                Err(_) => -1,
            }
        }
    };
    expanded.into()
}
