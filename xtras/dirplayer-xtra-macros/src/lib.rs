use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    Attribute, FnArg, Ident, ImplItem, ImplItemFn, ItemImpl, ItemStruct, LitStr, Meta, Token,
};

// ── Helper ────────────────────────────────────────────────────────────────────

/// Extract the `#[handler("name")]` override from a method's attributes.
fn extract_handler_name(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if !attr.path().is_ident("handler") {
            continue;
        }
        if let Ok(Meta::List(ml)) = attr.parse_args::<Meta>() {
            // `#[handler("name")]` — the list contains the string literal
            if let Ok(lit) = syn::parse2::<LitStr>(ml.tokens.clone()) {
                return Some(lit.value().to_lowercase());
            }
        }
        // Direct `#[handler("name")]` — the arg itself is a LitStr
        if let Ok(lit) = attr.parse_args::<LitStr>() {
            return Some(lit.value().to_lowercase());
        }
    }
    None
}

// ── #[xtra_plugin("Name")] ────────────────────────────────────────────────────

/// Annotate a plugin struct to generate all required C ABI exports.
///
/// The annotated struct must implement `dirplayer_xtra::XtraPlugin`.
///
/// ```rust,ignore
/// #[xtra_plugin("MyXtra")]
/// struct MyPlugin;
/// ```
#[proc_macro_attribute]
pub fn xtra_plugin(attr: TokenStream, item: TokenStream) -> TokenStream {
    let xtra_name_lit = parse_macro_input!(attr as LitStr);
    let xtra_name = xtra_name_lit.value();
    let struct_item = parse_macro_input!(item as ItemStruct);
    let struct_name = &struct_item.ident;
    let name_bytes = xtra_name.as_bytes().to_vec();
    let name_len = name_bytes.len() as i32;

    let expanded = quote! {
        #struct_item

        // Resolved at compile time from the XtraPlugin impl.
        type __XtraInstance = <#struct_name as dirplayer_xtra::XtraPlugin>::Instance;

        // ── Instance registry ─────────────────────────────────────────────
        mod __xtra_registry {
            use std::collections::HashMap;
            use std::cell::RefCell;
            use super::#struct_name;

            thread_local! {
                pub static PLUGIN: RefCell<#struct_name> = RefCell::new(#struct_name);
                pub static INSTANCES: RefCell<HashMap<u32, super::__XtraInstance>> =
                    RefCell::new(HashMap::new());
                pub static COUNTER: std::cell::Cell<u32> = std::cell::Cell::new(1);
                pub static RESULT_BUF: RefCell<Vec<u8>> = RefCell::new(Vec::new());
                pub static ERROR_BUF: RefCell<Vec<u8>> = RefCell::new(Vec::new());
            }
        }

        // ── Memory management exports ─────────────────────────────────────
        #[no_mangle]
        pub unsafe extern "C" fn alloc(size: i32) -> *mut u8 {
            let mut v = Vec::<u8>::with_capacity(size as usize);
            let ptr = v.as_mut_ptr();
            std::mem::forget(v);
            ptr
        }

        #[no_mangle]
        pub unsafe extern "C" fn dealloc(ptr: *mut u8, size: i32) {
            drop(Vec::from_raw_parts(ptr, 0, size as usize));
        }

        // ── xtra name ─────────────────────────────────────────────────────
        static __XTRA_NAME: &[u8] = &[#(#name_bytes),*];

        #[no_mangle]
        pub extern "C" fn xtra_name_ptr() -> *const u8 {
            __XTRA_NAME.as_ptr()
        }

        #[no_mangle]
        pub extern "C" fn xtra_name_len() -> i32 {
            #name_len
        }

        // ── create / destroy instance ─────────────────────────────────────
        #[no_mangle]
        pub extern "C" fn xtra_create_instance(args_ptr: *const u8, args_len: i32) -> i32 {
            let args_json = unsafe {
                std::str::from_utf8_unchecked(
                    std::slice::from_raw_parts(args_ptr, args_len as usize)
                )
            };
            let args = match dirplayer_xtra::args_from_json(args_json) {
                Ok(a) => a,
                Err(e) => {
                    __xtra_registry::ERROR_BUF.with(|b| *b.borrow_mut() = e.into_bytes());
                    return -1;
                }
            };
            let result = __xtra_registry::PLUGIN.with(|p| {
                use dirplayer_xtra::XtraPlugin;
                p.borrow_mut().create(&args)
            });
            match result {
                Ok(instance) => {
                    let id = __xtra_registry::COUNTER.with(|c| {
                        let v = c.get();
                        c.set(v + 1);
                        v
                    });
                    __xtra_registry::INSTANCES.with(|m| m.borrow_mut().insert(id, instance));
                    id as i32
                }
                Err(e) => {
                    __xtra_registry::ERROR_BUF.with(|b| *b.borrow_mut() = e.into_bytes());
                    -1
                }
            }
        }

        #[no_mangle]
        pub extern "C" fn xtra_destroy_instance(id: i32) {
            __xtra_registry::INSTANCES.with(|m| m.borrow_mut().remove(&(id as u32)));
        }

        // ── handler dispatch ──────────────────────────────────────────────
        #[no_mangle]
        pub extern "C" fn xtra_call_handler(
            id: i32,
            name_ptr: *const u8,
            name_len: i32,
            args_ptr: *const u8,
            args_len: i32,
        ) -> i32 {
            let handler_name = unsafe {
                std::str::from_utf8_unchecked(
                    std::slice::from_raw_parts(name_ptr, name_len as usize)
                )
            };
            let args_json = unsafe {
                std::str::from_utf8_unchecked(
                    std::slice::from_raw_parts(args_ptr, args_len as usize)
                )
            };
            let args = match dirplayer_xtra::args_from_json(args_json) {
                Ok(a) => a,
                Err(e) => {
                    __xtra_registry::ERROR_BUF.with(|b| *b.borrow_mut() = e.into_bytes());
                    return -1;
                }
            };
            let result = __xtra_registry::INSTANCES.with(|m| {
                let mut map = m.borrow_mut();
                match map.get_mut(&(id as u32)) {
                    Some(instance) => __dispatch_handler(instance, handler_name, &args),
                    None => Err(format!("Instance {} not found", id)),
                }
            });
            match result {
                Ok(datum) => {
                    let json = dirplayer_xtra::datum_to_json(&datum);
                    __xtra_registry::RESULT_BUF.with(|b| *b.borrow_mut() = json.into_bytes());
                    0
                }
                Err(e) => {
                    __xtra_registry::ERROR_BUF.with(|b| *b.borrow_mut() = e.into_bytes());
                    -1
                }
            }
        }

        // ── static handler dispatch ───────────────────────────────────────
        #[no_mangle]
        pub extern "C" fn xtra_call_static_handler(
            name_ptr: *const u8,
            name_len: i32,
            args_ptr: *const u8,
            args_len: i32,
        ) -> i32 {
            let handler_name = unsafe {
                std::str::from_utf8_unchecked(
                    std::slice::from_raw_parts(name_ptr, name_len as usize)
                )
            };
            let args_json = unsafe {
                std::str::from_utf8_unchecked(
                    std::slice::from_raw_parts(args_ptr, args_len as usize)
                )
            };
            let args = match dirplayer_xtra::args_from_json(args_json) {
                Ok(a) => a,
                Err(e) => {
                    __xtra_registry::ERROR_BUF.with(|b| *b.borrow_mut() = e.into_bytes());
                    return -1;
                }
            };
            let result = __xtra_registry::PLUGIN.with(|p| {
                __dispatch_static_handler(&mut *p.borrow_mut(), handler_name, &args)
            });
            match result {
                Ok(datum) => {
                    let json = dirplayer_xtra::datum_to_json(&datum);
                    __xtra_registry::RESULT_BUF.with(|b| *b.borrow_mut() = json.into_bytes());
                    0
                }
                Err(e) => {
                    __xtra_registry::ERROR_BUF.with(|b| *b.borrow_mut() = e.into_bytes());
                    -1
                }
            }
        }

        // ── async / static query ──────────────────────────────────────────
        #[no_mangle]
        pub extern "C" fn xtra_has_async_handler(name_ptr: *const u8, name_len: i32) -> i32 {
            let _ = (name_ptr, name_len);
            0 // async not supported in v1
        }

        #[no_mangle]
        pub extern "C" fn xtra_has_static_handler(name_ptr: *const u8, name_len: i32) -> i32 {
            let name = unsafe {
                std::str::from_utf8_unchecked(
                    std::slice::from_raw_parts(name_ptr, name_len as usize)
                )
            };
            if __has_static_handler(name) { 1 } else { 0 }
        }

        // ── result / error buffers ────────────────────────────────────────
        #[no_mangle]
        pub extern "C" fn xtra_get_result_ptr() -> *const u8 {
            __xtra_registry::RESULT_BUF.with(|b| b.borrow().as_ptr())
        }

        #[no_mangle]
        pub extern "C" fn xtra_get_result_len() -> i32 {
            __xtra_registry::RESULT_BUF.with(|b| b.borrow().len() as i32)
        }

        #[no_mangle]
        pub extern "C" fn xtra_get_error_ptr() -> *const u8 {
            __xtra_registry::ERROR_BUF.with(|b| b.borrow().as_ptr())
        }

        #[no_mangle]
        pub extern "C" fn xtra_get_error_len() -> i32 {
            __xtra_registry::ERROR_BUF.with(|b| b.borrow().len() as i32)
        }
    };

    TokenStream::from(expanded)
}

// ── #[xtra_handlers] ──────────────────────────────────────────────────────────

/// Generate an instance-handler dispatch function for an impl block.
///
/// Each `pub fn` in the impl becomes a match arm in the generated
/// `__dispatch_handler(instance, name, args)` function.  Handler names are
/// matched case-insensitively against the already-lowercased `name` argument.
///
/// Use `#[handler("exactName")]` to override the match key for a specific
/// method (the provided name is also lowercased before comparison).
///
/// ```rust,ignore
/// #[xtra_handlers]
/// impl MyInstance {
///     fn get_value(&mut self) -> Result<Datum, String> { ... }
///
///     #[handler("getValue")]
///     fn get_value_alias(&mut self, args: &[Datum]) -> Result<Datum, String> { ... }
/// }
/// ```
#[proc_macro_attribute]
pub fn xtra_handlers(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let impl_block = parse_macro_input!(item as ItemImpl);
    let self_ty = &impl_block.self_ty;

    let mut match_arms = Vec::new();

    for impl_item in &impl_block.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };
        // Skip non-pub methods.
        if !matches!(method.vis, syn::Visibility::Public(_)) {
            continue;
        }

        let match_key = extract_handler_name(&method.attrs)
            .unwrap_or_else(|| method.sig.ident.to_string().to_lowercase());

        let method_ident = &method.sig.ident;
        let takes_args = method.sig.inputs.len() > 1; // first is self

        let call = if takes_args {
            quote! { instance.#method_ident(args) }
        } else {
            quote! { instance.#method_ident() }
        };

        match_arms.push(quote! {
            #match_key => #call,
        });
    }

    let expanded = quote! {
        #impl_block

        fn __dispatch_handler(
            instance: &mut #self_ty,
            name: &str,
            args: &[dirplayer_xtra::Datum],
        ) -> Result<dirplayer_xtra::Datum, String> {
            match name {
                #(#match_arms)*
                other => Err(format!("Unknown handler: {}", other)),
            }
        }
    };

    TokenStream::from(expanded)
}

// ── #[xtra_static_handlers] ───────────────────────────────────────────────────

/// Generate a static-handler dispatch function for an impl block on the plugin
/// struct.  Same naming rules as `#[xtra_handlers]`.
///
/// ```rust,ignore
/// #[xtra_static_handlers]
/// impl MyPlugin {
///     pub fn get_version(&mut self) -> Result<Datum, String> {
///         Ok("1.0".into())
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn xtra_static_handlers(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let impl_block = parse_macro_input!(item as ItemImpl);
    let self_ty = &impl_block.self_ty;

    let mut match_arms = Vec::new();
    let mut handler_names = Vec::<String>::new();

    for impl_item in &impl_block.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };
        if !matches!(method.vis, syn::Visibility::Public(_)) {
            continue;
        }

        let match_key = extract_handler_name(&method.attrs)
            .unwrap_or_else(|| method.sig.ident.to_string().to_lowercase());

        handler_names.push(match_key.clone());
        let method_ident = &method.sig.ident;
        let takes_args = method.sig.inputs.len() > 1;

        let call = if takes_args {
            quote! { plugin.#method_ident(args) }
        } else {
            quote! { plugin.#method_ident() }
        };

        match_arms.push(quote! {
            #match_key => #call,
        });
    }

    let expanded = quote! {
        #impl_block

        fn __dispatch_static_handler(
            plugin: &mut #self_ty,
            name: &str,
            args: &[dirplayer_xtra::Datum],
        ) -> Result<dirplayer_xtra::Datum, String> {
            match name {
                #(#match_arms)*
                other => Err(format!("Unknown static handler: {}", other)),
            }
        }

        fn __has_static_handler(name: &str) -> bool {
            matches!(name, #(#handler_names)|*)
        }
    };

    TokenStream::from(expanded)
}
