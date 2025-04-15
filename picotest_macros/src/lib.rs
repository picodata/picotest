mod utils;

use darling::ast::NestedMeta;
use darling::{Error, FromMeta};
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, parse_quote, Item};

fn plugin_path_default() -> String {
    ".".to_string()
}
fn plugin_timeout_secs_default() -> u64 {
    5
}
#[derive(Debug, FromMeta)]
struct PluginCfg {
    #[darling(default = "plugin_path_default")]
    path: String,
    #[darling(default = "plugin_timeout_secs_default")]
    timeout: u64,
}

#[proc_macro_attribute]
pub fn picotest(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as Item);

    let attr = match NestedMeta::parse_meta_list(attr.into()) {
        Ok(v) => v,
        Err(e) => {
            return TokenStream::from(Error::from(e).write_errors());
        }
    };

    let cfg = match PluginCfg::from_list(&attr) {
        Ok(v) => v,
        Err(e) => {
            return TokenStream::from(e.write_errors());
        }
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
            let test_runner_ident = format_ident!("{}_", test_fn.sig.ident);
            let test_runner = quote! {
                #[test]
                fn #test_runner_ident() {
                    use picotest::internal;

                    let plugin_path = std::env::current_dir()
                        .expect("Failed to obtain current directory");
                    let plugin_dylib_path =
                        internal::plugin_dylib_path(&plugin_path);

                    let call_test_fn_query =
                        internal::lua_ffi_call_unit_test(
                            #test_fn_name, plugin_dylib_path.to_str().unwrap());

                    let cluster = picotest::cluster(plugin_path.to_str().unwrap(), 0);
                    let output = cluster.run_query(call_test_fn_query)
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
