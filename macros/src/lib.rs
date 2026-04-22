extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, parse_quote, Attribute, FnArg, ImplItem, ItemFn, ItemImpl, ItemStruct,
    ReturnType,
};

#[proc_macro_derive(SerDe)]
pub fn serialize_deserialize_derive(input: TokenStream) -> TokenStream {
    use quote::quote;
    use syn::{parse_macro_input, Data, DeriveInput, Fields, Type};

    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => panic!("SerDe can only be derived for structs with named fields"),
        },
        _ => panic!("SerDe can only be derived for structs"),
    };

    let field_names: Vec<_> = fields.iter().map(|f| f.ident.as_ref().unwrap()).collect();
    let field_strs: Vec<_> = field_names.iter().map(|f| f.to_string()).collect();
    let field_types: Vec<_> = fields.iter().map(|f| &f.ty).collect();

    fn is_optional(ty: &Type) -> bool {
        if let Type::Path(tp) = ty {
            if let Some(seg) = tp.path.segments.first() {
                return seg.ident == "Option";
            }
        }
        false
    }

    let mut ser_generics = input.generics.clone();
    for param in &mut ser_generics.params {
        if let syn::GenericParam::Type(type_param) = param {
            type_param.bounds.push(parse_quote!(Serialize));
        }
    }

    let mut de_generics = input.generics.clone();
    for param in &mut de_generics.params {
        if let syn::GenericParam::Type(type_param) = param {
            type_param.bounds.push(parse_quote!(Deserialize));
        }
    }

    let (ser_impl, ser_ty, ser_where) = ser_generics.split_for_impl();
    let (de_impl, de_ty, de_where) = de_generics.split_for_impl();

    let serialize_fields = field_names
        .iter()
        .zip(field_strs.iter())
        .map(|(name, key)| {
            quote! {
                if let Some(val) = {
                    let serialized = self.#name.serialize();
                    if serialized != "None" {
                        Some(format!("{}={}", #key, serialized))
                    } else { None }
                } {
                    parts.push(val);
                }
            }
        });

    let deserialize_fields = field_names
        .iter()
        .zip(field_strs.iter())
        .zip(field_types.iter())
        .map(|((name, key), ty)| {
            let is_opt = is_optional(ty);
            if is_opt {
                quote! {
                    #name: {
                        if let Some(val) = map.get(#key) {
                            <#ty>::deserialize(val)?
                        } else {
                            None
                        }
                    }
                }
            } else {
                quote! {
                    #name: {
                        let val = map.get(#key)
                            .ok_or_else(|| format!("Missing field '{}'", #key))?;
                        <#ty>::deserialize(val)?
                    }
                }
            }
        });

    // let name_with_generics = format!("{}{}", name, generics);
    let output = quote! {
        impl #ser_impl Serialize for #name #ser_ty #ser_where {
            fn serialize(&self) -> String {
                let mut parts = Vec::new();
                #(#serialize_fields)*
                format!("{{{}}}", parts.join(" "))
            }
        }

        impl #de_impl Deserialize for #name #de_ty #de_where {
            fn deserialize(s: &str) -> Result<Self, String> {
                let s = s.trim();
                let inner = if s.starts_with('{') && s.ends_with('}') {
                    &s[1..s.len()-1]
                } else {
                    s
                };
                let map = crate::rpc::parse_key_values(inner)?;
                Ok(Self {
                    #(#deserialize_fields),*
                })
            }
        }
    };

    TokenStream::from(output)
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
        _ => panic!("Expected a return type of `Result<Res, RpcError>`"),
    };

    let output = if let Some(self_arg) = self_arg {
        quote! {
            fn #fn_name(#self_arg, req: String) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<String, RpcError>> + Send>> {
                Box::pin(async move {
                    let req = #req_type::deserialize(req.as_str())
                        .map_err(|e| RpcError::Deserialize(e))?;

                    let result: #output_type = #fn_body;

                    result.map(|res| res.serialize())
                })
            }
        }
    } else {
        quote! {
            fn #fn_name(req: String) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<String, RpcError>> + Send>> {
                Box::pin(async move {
                    let req = #req_type::deserialize(req.as_str())
                        .map_err(|e| RpcError::Deserialize(e))?;

                    let result: #output_type = #fn_body;

                    result.map(|res| res.serialize())
                })

            }
        }
    };

    TokenStream::from(output)
}

#[proc_macro_attribute]
pub fn rpc_stream(_attr: TokenStream, item: TokenStream) -> TokenStream {
    use quote::quote;
    use syn::{parse_macro_input, FnArg, ItemFn, ReturnType};

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
        _ => panic!("Expected typed argument for req"),
    };

    let output_type = match &input.sig.output {
        ReturnType::Type(_, ty) => ty,
        _ => panic!("Expected a return type of `Result<mpsc::Receiver<Res>, RpcError>`"),
    };

    let output = if let Some(self_arg) = self_arg {
        quote! {
            fn #fn_name(#self_arg, req: String)
                -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<::tokio::sync::mpsc::Receiver<String>, RpcError>> + Send>>
            {
                Box::pin(async move {
                     let req = #req_type::deserialize(req.as_str())
                        .map_err(|e| RpcError::Deserialize(e))?;

                    let result: #output_type = #fn_body;
                    result.map(|rx| {
                        let (tx2, rx2) = ::tokio::sync::mpsc::channel(10);
                        ::tokio::spawn(async move {
                            let mut rx = rx;
                            while let Some(item) = rx.recv().await {
                                let _ = tx2.send(item.serialize()).await;
                            }
                            let _ = tx2.send("done".to_string()).await;
                            drop(tx2);
                        });
                        rx2
                    })
                })
            }
        }
    } else {
        quote! {
            fn #fn_name(req: String)
                -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<::tokio::sync::mpsc::Receiver<String>, RpcError>> + Send>>
            {
                Box::pin(async move {
                    let req = #req_type::deserialize(req.as_str())
                        .map_err(|e| RpcError::Deserialize(e))?;

                    let result: #output_type = #fn_body;
                    result.map(|rx| {
                        let (tx2, rx2) = ::tokio::sync::mpsc::channel(10);
                        ::tokio::spawn(async move {
                            let mut rx = rx;
                            while let Some(item) = rx.recv().await {
                                let _ = tx2.send(item.serialize()).await;
                            }
                            let _ = tx2.send("done".to_string()).await;
                            drop(tx2);
                        });
                        rx2
                    })
                })
            }
        }
    };

    TokenStream::from(output)
}

#[proc_macro_attribute]
pub fn rpc_struct(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);

    let expanded = quote! {
        #input
    };

    expanded.into()
}

#[proc_macro_attribute]
pub fn rpc_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemImpl);

    let ty = &input.self_ty;

    let mut rpc_methods = Vec::new();

    for item in &input.items {
        if let ImplItem::Fn(method) = item {
            let method_name = &method.sig.ident;
            let method_name_str = method_name.to_string();

            if has_rpc_func_attr(&method.attrs) {
                let requires_leader = has_requires_leader_attr(&method.attrs);
                let leader_check = if requires_leader {
                    quote! {
                        if !crate::consensus::ConsensusHandle::is_leader() {
                            let leader = crate::consensus::ConsensusHandle::leader_id()
                                .unwrap_or_else(|| "unknown".to_string());
                            return Err(RpcError::NotLeader(leader));
                        }
                    }
                } else {
                    quote! {}
                };

                rpc_methods.push(quote! {
                    dispatcher.register_fn(
                        format!("{}", #method_name_str),
                        std::sync::Arc::new(move |req: String| {
                            Box::pin(async move {
                                #leader_check
                                self.#method_name(req).await
                            })
                        }),
                    );
                });
            } else if has_rpc_stream_attr(&method.attrs) {
                let requires_leader = has_requires_leader_attr(&method.attrs);
                let leader_check = if requires_leader {
                    quote! {
                        if !crate::consensus::ConsensusHandle::is_leader() {
                            let leader = crate::consensus::ConsensusHandle::leader_id()
                                .unwrap_or_else(|| "unknown".to_string());
                            return Err(RpcError::NotLeader(leader));
                        }
                    }
                } else {
                    quote! {}
                };

                rpc_methods.push(quote! {
                    dispatcher.register_stream_fn(
                        format!("{}", #method_name_str),
                        std::sync::Arc::new(move |req: String| {
                            Box::pin(async move {
                                #leader_check
                                self.#method_name(req).await
                            })
                        }),
                    );
                });
            }
        }
    }

    fn has_rpc_stream_attr(attrs: &[Attribute]) -> bool {
        attrs.iter().any(|attr| attr.path().is_ident("rpc_stream"))
    }

    let expanded = quote! {
        #input

        impl RpcStruct for #ty {
            fn register_fns(&'static self, dispatcher: &mut Dispatcher) {
                #( #rpc_methods )*
            }
        }
    };

    expanded.into()
}

fn has_rpc_func_attr(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("rpc_func"))
}

fn has_requires_leader_attr(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("requires_leader"))
}

#[proc_macro_attribute]
pub fn requires_leader(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
