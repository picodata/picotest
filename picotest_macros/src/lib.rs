mod utils;

use darling::ast::NestedMeta;
use darling::{Error, FromMeta};
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, parse_quote, Attribute, Item, Stmt};
use utils::traverse_use_item;

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

    let run_cluster_call = quote! {
        picotest::run_cluster(
            #path,
            #timeout_secs,
        ).expect("Failed to start the cluster")
    }
    .into_iter();

    let rstest_macro: Attribute = parse_quote! { #[rstest] };
    let input = match input {
        Item::Fn(mut func) => {
            let run_cluster: Stmt = parse_quote! {
                let mut cluster = #(#run_cluster_call)*;
            };

            func.attrs.insert(0, rstest_macro.clone());
            let mut stmts = vec![run_cluster];
            stmts.append(&mut func.block.stmts);
            func.block.stmts = stmts;
            Item::Fn(func)
        }
        Item::Mod(mut m) => {
            let (brace, items) = m.content.clone().unwrap();

            let run_cluster: Stmt = parse_quote! {
                let mut cluster = CLUSTER.get_or_init(|| {
                    #(#run_cluster_call)*
                });
            };

            let resume: Stmt = parse_quote! {
                if let Err(err) = result {
                    panic::resume_unwind(err);
                }
            };

            let mut has_once_lock: bool = false;
            let mut use_atomic_usize: bool = false;
            let mut use_atomic_ordering: bool = false;
            let mut use_panic: bool = false;
            let mut e: Vec<Item> = items
                .into_iter()
                .map(|t| match t {
                    Item::Fn(mut func) => {
                        let func_name = &func.sig.ident;
                        if func_name.to_string().starts_with("test_") {
                            func.attrs.insert(0, rstest_macro.clone());
                            let block = func.block.clone();
                            let body: Stmt = parse_quote! {
                                let result = panic::catch_unwind(
                                // Cluster carries JoinHandles's of spawned instances,
                                // which are not unwind safe. So far explicitly wrap it
                                // into AssertUnwindSafe to pass it though catch_unwind / resume_unwind
                                // routines, which require objects to implement UnwindSafe trait.
                                //
                                // TODO: wrap cluster into Mutex or other sync wrapper.
                                panic::AssertUnwindSafe(|| {
                                    #block
                                }));
                            };

                            func.block.stmts = vec![run_cluster.clone(), body, resume.clone()];
                            Item::Fn(func)
                        } else {
                            Item::Fn(func)
                        }
                    }
                    Item::Use(use_stmt) => {
                        if traverse_use_item(&use_stmt.tree, vec!["std", "sync", "OnceLock"])
                            .is_some()
                        {
                            has_once_lock = true;
                        }
                        if traverse_use_item(
                            &use_stmt.tree,
                            vec!["std", "sync", "atomic", "AtomicUsize"],
                        )
                        .is_some()
                        {
                            use_atomic_usize = true;
                        }
                        if traverse_use_item(
                            &use_stmt.tree,
                            vec!["std", "sync", "atomic", "Ordering"],
                        )
                        .is_some()
                        {
                            use_atomic_ordering = true;
                        }
                        if traverse_use_item(&use_stmt.tree, vec!["std", "panic"]).is_some() {
                            use_panic = true;
                        }
                        Item::Use(use_stmt)
                    }
                    e => e,
                })
                .collect();

            let mut use_content = vec![parse_quote!(
                use picotest::*;
            )];
            if !has_once_lock {
                use_content.push(parse_quote!(
                    use std::sync::OnceLock;
                ));
            }

            if !use_atomic_usize {
                use_content.push(parse_quote!(
                    use std::sync::atomic::AtomicUsize;
                ));
            }
            if !use_atomic_ordering {
                use_content.push(parse_quote!(
                    use std::sync::atomic::Ordering;
                ));
            }

            if !use_panic {
                use_content.push(parse_quote!(
                    use std::panic;
                ));
            }

            use_content.push(parse_quote!(
                static CLUSTER: OnceLock<Cluster> = OnceLock::new();
            ));
            use_content.append(&mut e);

            let mut tear_down_fn = vec![parse_quote! {
                #[ctor::dtor]
                fn tear_down() {
                    if let Some(cluster) = CLUSTER.get() {
                        cluster.stop();
                    }
                }
            }];
            use_content.append(&mut tear_down_fn);

            m.content = Some((brace, use_content));
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

                    let cluster = picotest::run_cluster(plugin_path.to_str().unwrap(), 0)
                        .expect(concat!("Failed to spin up the cluster for running unit-test '", #test_fn_name, "'"));
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
