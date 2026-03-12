use proc_macro::TokenStream;

use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Error, Expr, Ident, ItemFn, LitStr, Result, Token, parse_macro_input};

#[proc_macro_attribute]
pub fn get(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand_route_macro("GET", attr, item)
}

#[proc_macro_attribute]
pub fn post(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand_route_macro("POST", attr, item)
}

#[proc_macro_derive(RsqSchema)]
pub fn derive_schema(_item: TokenStream) -> TokenStream {
    TokenStream::new()
}

struct RouteArgs {
    path: LitStr,
    summary: Option<LitStr>,
    description: Option<LitStr>,
    operation_id: Option<LitStr>,
    tags: Vec<LitStr>,
}

impl Parse for RouteArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let path = input.parse::<LitStr>()?;
        let mut summary = None;
        let mut description = None;
        let mut operation_id = None;
        let mut tags = Vec::new();

        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }

            let key = input.parse::<Ident>()?;
            input.parse::<Token![=]>()?;
            let expr = input.parse::<Expr>()?;
            let value = match expr {
                Expr::Lit(expr_lit) => match expr_lit.lit {
                    syn::Lit::Str(value) => value,
                    other => {
                        return Err(Error::new_spanned(other, "expected string literal"));
                    }
                },
                other => {
                    return Err(Error::new_spanned(other, "expected string literal"));
                }
            };

            match key.to_string().as_str() {
                "summary" => summary = Some(value),
                "description" => description = Some(value),
                "operation_id" => operation_id = Some(value),
                "tag" => tags.push(value),
                _ => {
                    return Err(Error::new_spanned(
                        key,
                        "unsupported route macro option",
                    ));
                }
            }
        }

        Ok(Self {
            path,
            summary,
            description,
            operation_id,
            tags,
        })
    }
}

fn expand_route_macro(method: &str, attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as RouteArgs);
    let function = parse_macro_input!(item as ItemFn);

    let fn_name = &function.sig.ident;
    let visibility = &function.vis;
    let route_fn_name = format_ident!("{}_route", fn_name);
    let method_ident = format_ident!("{method}");
    let doc = format!("Generated FastRust route for `{fn_name}`.");

    let summary = args.summary.map(|value| quote! { meta.set_summary(#value); });
    let description = args
        .description
        .map(|value| quote! { meta.set_description(#value); });
    let operation_id = args
        .operation_id
        .map(|value| quote! { meta.set_operation_id(#value); });
    let tags = args
        .tags
        .into_iter()
        .map(|value| quote! { meta.add_tag(#value); })
        .collect::<Vec<_>>();
    let path = args.path;

    TokenStream::from(quote! {
        #function

        #[doc = #doc]
        #visibility fn #route_fn_name() -> ::rust_squared::Route {
            let mut meta = ::rust_squared::RouteMeta::default();
            #summary
            #description
            #operation_id
            #(#tags)*
            ::rust_squared::route_with_meta(::rust_squared::Method::#method_ident, #path, #fn_name, meta)
        }
    })
}
