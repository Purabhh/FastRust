use proc_macro::TokenStream;

use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Data, DeriveInput, Error, Expr, Fields, GenericArgument, Ident, ItemFn, LitStr, PathArguments, Result, Token, Type, parse_macro_input};

#[proc_macro_attribute]
pub fn get(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand_route_macro("GET", attr, item)
}

#[proc_macro_attribute]
pub fn post(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand_route_macro("POST", attr, item)
}

#[proc_macro_derive(RsqSchema)]
pub fn derive_schema(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    expand_schema_derive(input).into()
}

struct RouteArgs {
    path: LitStr,
    summary: Option<LitStr>,
    description: Option<LitStr>,
    operation_id: Option<LitStr>,
    tags: Vec<LitStr>,
    request_body: Option<LitStr>,
    response: Option<LitStr>,
}

fn expand_schema_derive(input: DeriveInput) -> proc_macro2::TokenStream {
    let name = input.ident;
    let schema_name = name.to_string();

    let fields = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => fields.named,
            other => {
                return Error::new_spanned(other, "RsqSchema only supports structs with named fields")
                    .to_compile_error();
            }
        },
        _ => {
            return Error::new_spanned(&name, "RsqSchema only supports structs").to_compile_error();
        }
    };

    let properties = fields.iter().map(|field| {
        let field_name = field.ident.as_ref().expect("named field").to_string();
        let schema_tokens = schema_for_type(&field.ty);
        quote! {
            properties.insert(#field_name.to_string(), #schema_tokens);
        }
    });

    let required = fields.iter().filter_map(|field| {
        if is_option(&field.ty) {
            None
        } else {
            Some(field.ident.as_ref().expect("named field").to_string())
        }
    });

    quote! {
        impl ::rust_squared::schema::RsqSchema for #name {
            fn schema_name() -> &'static str {
                #schema_name
            }

            fn schema() -> ::rust_squared::serde_json::Value {
                let mut properties = ::rust_squared::serde_json::Map::new();
                #(#properties)*
                ::rust_squared::serde_json::json!({
                    "type": "object",
                    "properties": properties,
                    "required": [#(#required),*]
                })
            }
        }
    }
}

fn schema_for_type(ty: &Type) -> proc_macro2::TokenStream {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            let ident = segment.ident.to_string();
            return match ident.as_str() {
                "String" | "str" => quote! { ::rust_squared::serde_json::json!({ "type": "string" }) },
                "bool" => quote! { ::rust_squared::serde_json::json!({ "type": "boolean" }) },
                "u8" | "u16" | "u32" | "u64" | "usize" => {
                    quote! { ::rust_squared::serde_json::json!({ "type": "integer", "format": "uint64" }) }
                }
                "i8" | "i16" | "i32" | "i64" | "isize" => {
                    quote! { ::rust_squared::serde_json::json!({ "type": "integer", "format": "int64" }) }
                }
                "f32" | "f64" => quote! { ::rust_squared::serde_json::json!({ "type": "number", "format": "double" }) },
                "Vec" => {
                    if let PathArguments::AngleBracketed(args) = &segment.arguments {
                        if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                            let inner = schema_for_type(inner_ty);
                            return quote! { ::rust_squared::serde_json::json!({ "type": "array", "items": #inner }) };
                        }
                    }
                    quote! { ::rust_squared::serde_json::json!({ "type": "array" }) }
                }
                "Option" => {
                    if let PathArguments::AngleBracketed(args) = &segment.arguments {
                        if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                            let inner = schema_for_type(inner_ty);
                            return quote! {{
                                let mut value = #inner;
                                if let Some(object) = value.as_object_mut() {
                                    object.insert("nullable".to_string(), ::rust_squared::serde_json::json!(true));
                                }
                                value
                            }};
                        }
                    }
                    quote! { ::rust_squared::serde_json::json!({}) }
                }
                _ => quote! {{
                    ::rust_squared::serde_json::json!({ "$ref": format!("#/components/schemas/{}", <#ty as ::rust_squared::schema::RsqSchema>::schema_name()) })
                }},
            };
        }
    }

    quote! { ::rust_squared::serde_json::json!({}) }
}

fn is_option(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            return segment.ident == "Option";
        }
    }
    false
}

impl Parse for RouteArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let path = input.parse::<LitStr>()?;
        let mut summary = None;
        let mut description = None;
        let mut operation_id = None;
        let mut tags = Vec::new();
        let mut request_body = None;
        let mut response = None;

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
                "request_body" => request_body = Some(value),
                "response" => response = Some(value),
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
            request_body,
            response,
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
    let request_body = args
        .request_body
        .map(|value| quote! { meta.set_request_body_schema(#value); });
    let response = args
        .response
        .map(|value| quote! { meta.set_response_schema(#value); });
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
            #request_body
            #response
            ::rust_squared::route_with_meta(::rust_squared::Method::#method_ident, #path, #fn_name, meta)
        }
    })
}
