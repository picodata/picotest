mod utils;

use darling::ast::NestedMeta;
use darling::{Error, FromMeta};
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, parse_quote, Ident, Item};

fn plugin_timeout_secs_default() -> u64 {
    5
}

fn parse_attrs<T: FromMeta>(attr: TokenStream) -> Result<T, TokenStream> {
    NestedMeta::parse_meta_list(attr.into())
        .map_err(Error::from)
        .and_then(|attr| T::from_list(&attr))
        .map_err(|e| TokenStream::from(e.write_errors()))
}

#[derive(Debug, FromMeta)]
struct PluginCfg {
    path: Option<String>,
    #[darling(default = "plugin_timeout_secs_default")]
    timeout: u64,
}

#[proc_macro_attribute]
pub fn picotest(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as Item);
    let cfg: PluginCfg = match parse_attrs(attr) {
        Ok(cfg) => cfg,
        Err(err) => return err,
    };

    let path = cfg.path;
    let timeout_secs = cfg.timeout;
    let input = match input {
        Item::Fn(func) => Item::Fn(utils::process_test_function(func, &path, timeout_secs)),
        Item::Mod(mut m) => {
            let (brace, items) = m.content.unwrap();
            let mut items: Vec<Item> = items
                .into_iter()
                .map(|item| {
                    if let Item::Fn(func) = item {
                        Item::Fn(utils::process_test_function(func, &path, timeout_secs))
                    } else {
                        item
                    }
                })
                .collect();

            let mut content = vec![parse_quote!(
                use picotest::*;
            )];
            content.push(parse_quote!(
                use std::panic;
            ));
            content.append(&mut items);

            m.content = Some((brace, content));
            Item::Mod(m)
        }
        _ => {
            panic!("The #[picotest] macro is only valid when called on a function or module.");
        }
    };
    TokenStream::from(quote! (#input))
}

static UNIT_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

#[proc_macro_attribute]
pub fn picotest_unit(_: TokenStream, tokens: TokenStream) -> TokenStream {
    match parse_macro_input!(tokens as Item) {
        Item::Fn(mut test_fn) => {
            let test_fn_name = test_fn.sig.ident.to_string();
            // We want test routine to be called through FFI.
            // So mark it as 'pub extern "C"'.
            test_fn.vis = parse_quote! { pub };
            test_fn.sig.abi = parse_quote! { extern "C" };
            // Set no mangle attribute to avoid spoiling of function signature.
            test_fn.attrs = vec![
                parse_quote! { #[allow(dead_code)]  },
                parse_quote! { #[unsafe(no_mangle)] },
            ];

            // Create test runner - it's a wrapper around main test function.
            // This wrapper will call main test routine in a Lua runtime running
            // inside picodata instance.
            let test_runner_ident = test_fn.sig.ident.clone();

            // Name of the function to be invoked on instance-side as test payload
            let test_idx = UNIT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Acquire);
            let ffi_test_callable = format!("test_impl_{test_idx}_{test_fn_name}");
            test_fn.sig.ident = Ident::new(&ffi_test_callable, test_fn.sig.ident.span());

            let test_runner = quote! {
                #[test]
                fn #test_runner_ident() {
                    use picotest::internal;

                    let plugin_path = internal::plugin_root_dir();
                    let plugin_dylib_path =
                        internal::plugin_dylib_path(&plugin_path);
                    let plugin_topology = internal::get_or_create_unit_test_topology();

                    let call_test_fn_query =
                        internal::lua_ffi_call_unit_test(
                            #ffi_test_callable, plugin_dylib_path.to_str().unwrap());

                    let cluster = picotest::get_or_create_session_cluster(
                        plugin_path.to_str().unwrap().into(),
                        plugin_topology.into(),
                        0
                    );

                    let output = cluster.run_lua(call_test_fn_query)
                        .expect("Failed to execute query");

                    if let Err(err) = internal::verify_unit_test_output(&output) {
                        for l in output.split("----") {
                            println!("[Lua] {l}")
                        }
                        panic!("Test '{}' exited with failure: {}", #test_fn_name, err);
                    }
                }
            };

            quote! {
                #test_fn
                #test_runner
            }
            .into()
        }
        _ => panic!("The #[picotest_unit] macro is only valid when called on a function."),
    }
}
