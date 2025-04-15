use syn::{parse_quote, Attribute, FnArg, ItemFn, Stmt};
const TEST_PREFIX: &str = "test_";

pub fn process_test_function(mut func: ItemFn, path: &String, timeout: u64) -> ItemFn {
    let func_name = func.sig.ident.to_string();
    if !func_name.starts_with(TEST_PREFIX) {
        return func;
    }

    let rstest_macro: Attribute = parse_quote! { #[rstest] };
    func.attrs.insert(0, rstest_macro);

    let cluster: FnArg = parse_quote! {
        #[with(#path, #timeout)] cluster: &Cluster
    };
    func.sig.inputs.insert(0, cluster);

    let block = func.block.clone();
    let new_body: Stmt = parse_quote! {
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            #block
        }));
    };

    let resume: Stmt = parse_quote! {
        if let Err(err) = result {
            panic::resume_unwind(err);
        }
    };
    func.block.stmts = vec![new_body, resume];

    func
}
