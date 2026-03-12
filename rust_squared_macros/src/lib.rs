use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn get(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_derive(RsqSchema)]
pub fn derive_schema(_item: TokenStream) -> TokenStream {
    TokenStream::new()
}
