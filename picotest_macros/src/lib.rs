mod utils;

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::Nothing;
use syn::{parse, parse_macro_input, parse_quote, Ident, Item, ItemFn};

#[proc_macro_attribute]
pub fn picotest(attr: TokenStream, item: TokenStream) -> TokenStream {
    parse_macro_input!(attr as Nothing);
    let input = parse_macro_input!(item as Item);

    let input = match input {
        Item::Fn(func) => Item::Fn(utils::process_test_function(func)),
        Item::Mod(mut m) => {
            let (brace, items) = m.content.unwrap();
            let mut items: Vec<Item> = items
                .into_iter()
                .map(|item| {
                    if let Item::Fn(func) = item {
                        Item::Fn(utils::process_test_function(func))
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
            let test_fn_attrs = test_fn.attrs.clone();
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

            let tokens = quote! {
                #[test]
                fn #test_runner_ident() {
                    picotest::internal::run_unit_test(
                        #ffi_test_callable,
                        #test_fn_name,
                        env!("CARGO_PKG_NAME"),
                        env!("CARGO_MANIFEST_DIR"),
                    );
                }
            };

            let mut test_runner: ItemFn =
                parse(tokens.into()).expect("Runner routine tokens must be parsed");

            // Preserve attributes added to source test routine.
            test_runner.attrs.extend(test_fn_attrs);

            quote! {
                #test_fn
                #test_runner
            }
            .into()
        }
        _ => panic!("The #[picotest_unit] macro is only valid when called on a function."),
    }
}
