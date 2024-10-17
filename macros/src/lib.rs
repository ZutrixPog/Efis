extern crate proc_macro;
use proc_macro::TokenStream;

use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, FnArg, ItemFn, ReturnType};

#[proc_macro_derive(SerDe)]
pub fn serialize_deserialize_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let name = &input.ident;
    let data = &input.data;

    let expanded = match data {
        Data::Struct(ref data_struct) => match &data_struct.fields {
            Fields::Named(ref fields) => {
                let field_names: Vec<_> = fields.named.iter().map(|f| &f.ident).collect();

                let serialize_fields = field_names.iter().map(|name| {
                    quote! {
                        result.push(self.#name.to_string());
                    }
                });

                let deserialize_fields = field_names.iter().enumerate().map(|(i, name)| {

                    quote! {
                        let #name = parts[#i].parse().map_err(|_| format!("Failed to parse field '{}'", stringify!(#name)))?;

                    }
                });

                quote! {
                    impl Serialize for #name {
                        fn serialize(&self) -> String {
                            let mut result = Vec::new();
                            #(#serialize_fields)*
                            result.join(" ")
                        }
                    }

                    impl Deserialize for #name {
                        fn deserialize(s: &str) -> Result<Self, String> {
                            let parts: Vec<&str> = s.split_whitespace().collect();

                            #(#deserialize_fields)*

                            Ok(#name {
                                #(#field_names),*
                            })
                        }
                    }

                    impl SerDe for #name {
                        fn as_any(&self) -> Arc<dyn std::any::Any> {
                            Arc::new(self.clone())
                        }
                    }
                }
            }
            _ => panic!("SerializeDeserialize can only be derived for structs with named fields"),
        },
        _ => panic!("SerializeDeserialize can only be derived for structs"),
    };

    TokenStream::from(expanded)
}

#[proc_macro_attribute]
pub fn rpc_func(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let fn_name = &input.sig.ident;
    let fn_body = &input.block;

    let is_method = matches!(input.sig.inputs.first(), Some(FnArg::Receiver(_)));

    let (self_arg, req_arg) = if is_method {
        let self_arg = input.sig.inputs.first().unwrap();
        let req_arg = input
            .sig
            .inputs
            .iter()
            .nth(1)
            .expect("Expected method to have a second argument");
        (Some(self_arg), req_arg)
    } else {
        (
            None,
            input
                .sig
                .inputs
                .first()
                .expect("Expected function to have one argument"),
        )
    };

    let req_type = match req_arg {
        FnArg::Typed(arg) => &arg.ty,
        _ => panic!("Expected typed argument"),
    };

    let output_type = match &input.sig.output {
        ReturnType::Type(_, ty) => ty,
        _ => panic!("Expected a return type of `anyhow::Result<Res>`"),
    };

    let output = if let Some(self_arg) = self_arg {
        quote! {
            fn #fn_name(#self_arg, req: String) -> Pin<Box<dyn ::std::future::Future<Output = ::anyhow::Result<String>> + Send>> {
                let req = #req_type::deserialize(req.as_str()).unwrap();
                Box::pin(async move {
                    let result: #output_type = #fn_body;
                    result.map(|res| res.serialize())
                })
            }
        }
    } else {
        quote! {
            fn #fn_name(req: String) -> Pin<Box<dyn ::std::future::Future<Output = ::anyhow::Result<String>> + Send>> {
                let req = #req_type::deserialize(req.as_str()).unwrap();
                Box::pin(async move {
                    let result: #output_type = #fn_body;
                    result.map(|res| res.serialize())
                })
            }
        }
    };

    TokenStream::from(output)
}
