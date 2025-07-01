use proc_macro::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Fields, Meta, MetaNameValue, Type, parse_macro_input};

#[proc_macro_derive(AeadMessage, attributes(message_id, Aead, AeadSerialisation))]
pub fn derive_aead_message(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let message_id = extract_message_id(&input.attrs);
    let name = &input.ident;
    let fields = extract_struct_fields(&input.data);

    let (public_fields, private_fields) = categorize_fields(fields);

    let public_struct_name = if public_fields.is_empty() {
        syn::parse_str::<syn::Type>("()").unwrap()
    } else {
        let struct_name = syn::Ident::new(&format!("{}AssociatedData", name), name.span());
        syn::Type::Path(syn::TypePath {
            qself: None,
            path: struct_name.into(),
        })
    };

    let private_struct_name = if private_fields.is_empty() {
        syn::parse_str::<syn::Type>("()").unwrap()
    } else {
        let struct_name = syn::Ident::new(&format!("{}EncryptedData", name), name.span());
        syn::Type::Path(syn::TypePath {
            qself: None,
            path: struct_name.into(),
        })
    };

    let public_struct = generate_public_struct(&public_struct_name, &public_fields);
    let private_struct = generate_private_struct(&private_struct_name, &private_fields);
    let split_impl = generate_split_impl(
        name,
        &public_struct_name,
        &public_fields,
        &private_struct_name,
        &private_fields,
    );
    let from_parts_impl = generate_from_parts_impl(
        name,
        &public_struct_name,
        &public_fields,
        &private_struct_name,
        &private_fields,
    );

    let expanded = quote! {
        #public_struct
        #private_struct

        impl crate::codec::Message for #name {
            type AssociatedData = #public_struct_name;
            const MESSAGE_ID: u8 = #message_id as u8;
            #split_impl
            #from_parts_impl
        }
    };

    TokenStream::from(expanded)
}

fn extract_message_id(attrs: &[Attribute]) -> syn::Expr {
    let message_id_attrs: Vec<_> = attrs.iter().filter(|attr| attr.path().is_ident("message_id")).collect();

    match message_id_attrs.as_slice() {
        [] => panic!("message_id attribute is required"),
        [_, _, ..] => panic!("duplicate message_id attributes"),
        [attr] => match &attr.meta {
            Meta::Path(_) => panic!("message_id must be specified as message_id = N or message_id(expr)"),
            Meta::List(list) => {
                syn::parse2::<syn::Expr>(list.tokens.clone()).expect("Failed to parse message_id expression")
            }
            Meta::NameValue(MetaNameValue { value, .. }) => value.clone(),
        },
    }
}

fn extract_struct_fields(data: &Data) -> &syn::punctuated::Punctuated<syn::Field, syn::token::Comma> {
    match data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => panic!("Only named fields are supported"),
        },
        _ => panic!("Only structs are supported"),
    }
}

type FieldInfo<'a> = (&'a syn::Ident, &'a syn::Type, &'a [Attribute]);

fn categorize_fields(
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> (Vec<FieldInfo>, Vec<FieldInfo>) {
    let mut public_fields = Vec::new();
    let mut private_fields = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_type = &field.ty;

        let mut is_associated_data = false;
        let mut is_encrypted = false;

        for attr in &field.attrs {
            if attr.path().is_ident("Aead") {
                match &attr.meta {
                    Meta::List(list) => {
                        let tokens_str = list.tokens.to_string();
                        if tokens_str == "associated_data" {
                            is_associated_data = true;
                        } else if tokens_str == "encrypted" {
                            is_encrypted = true;
                        } else {
                            panic!(
                                "Unknown Aead attribute option '{}' for field {}",
                                tokens_str, field_name
                            );
                        }
                    }
                    _ => panic!(
                        "Aead attribute must be used as #[Aead(option)] for field {}",
                        field_name
                    ),
                }
            }
        }

        match (is_associated_data, is_encrypted) {
            (true, true) => panic!("Field {} cannot be both associated_data and encrypted", field_name),
            (true, false) => public_fields.push((field_name, field_type, field.attrs.as_slice())),
            (false, true) => private_fields.push((field_name, field_type, field.attrs.as_slice())),
            (false, false) => panic!(
                "Field {} must be marked as either #[Aead(associated_data)] or #[Aead(encrypted)]",
                field_name
            ),
        }
    }

    if public_fields.is_empty() && private_fields.is_empty() {
        panic!("Message must have at least one field");
    }

    (public_fields, private_fields)
}

fn extract_passthrough_attributes(attrs: &[Attribute]) -> Vec<proc_macro2::TokenStream> {
    attrs
        .iter()
        .filter_map(|attr| {
            if attr.path().is_ident("AeadSerialisation") {
                match &attr.meta {
                    Meta::List(list) => {
                        let tokens = &list.tokens;
                        Some(quote! { #[#tokens] })
                    }
                    _ => panic!("AeadSerialisation must be used as AeadSerialisation(attribute)"),
                }
            } else {
                None
            }
        })
        .collect()
}

fn generate_public_struct(public_struct_name: &Type, public_fields: &[FieldInfo]) -> proc_macro2::TokenStream {
    if public_fields.is_empty() {
        return quote! {};
    }

    let public_field_defs = public_fields.iter().map(|(name, ty, attrs)| {
        let passthrough_attrs = extract_passthrough_attributes(attrs);
        quote! { #(#passthrough_attrs)* pub #name: #ty }
    });

    quote! {
        #[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
        pub struct #public_struct_name {
            #(#public_field_defs),*
        }
    }
}

fn generate_private_struct(private_struct_name: &Type, private_fields: &[FieldInfo]) -> proc_macro2::TokenStream {
    if private_fields.is_empty() {
        return quote! {};
    }

    let private_field_defs = private_fields.iter().map(|(name, ty, attrs)| {
        let passthrough_attrs = extract_passthrough_attributes(attrs);
        quote! { #(#passthrough_attrs)* pub #name: #ty }
    });

    quote! {
        #[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
        pub(crate) struct #private_struct_name {
            #(#private_field_defs),*
        }
    }
}

fn generate_split_impl(
    name: &syn::Ident,
    public_struct_name: &Type,
    public_fields: &[FieldInfo],
    private_struct_name: &Type,
    private_fields: &[FieldInfo],
) -> proc_macro2::TokenStream {
    let public_data = if !public_fields.is_empty() {
        let field_assignments = public_fields.iter().map(|(name, _, _)| {
            quote! { #name: self.#name }
        });
        quote! {
            let public_data = #public_struct_name { #(#field_assignments),* };
            let public_bytes = bincode::encode_to_vec(&public_data, bincode::config::standard())?;
        }
    } else {
        quote! { let public_bytes : Vec<u8> = Vec::new(); }
    };

    let private_data = if !private_fields.is_empty() {
        let field_assignments = private_fields.iter().map(|(name, _, _)| {
            quote! { #name: self.#name }
        });
        quote! {
            let private_data = #private_struct_name { #(#field_assignments),* };
            let private_bytes = bincode::encode_to_vec(&private_data, bincode::config::standard())?;
        }
    } else {
        quote! { let private_bytes : Vec<u8> = Vec::new(); }
    };

    let message_parts = match (public_fields.is_empty(), private_fields.is_empty()) {
        (true, false) => quote! { crate::codec::MessageParts::PrivateOnly(private_bytes) },
        (false, true) => quote! { crate::codec::MessageParts::PublicOnly(public_bytes) },
        (false, false) => quote! {
            crate::codec::MessageParts::Both {
                public: public_bytes,
                private: private_bytes
            }
        },
        (true, true) => unreachable!(),
    };

    quote! {
        fn split(self) -> Result<crate::codec::MessageParts, crate::EncodeError> {
            #public_data
            #private_data
            Ok(#message_parts)
        }
    }
}

fn generate_from_parts_impl(
    name: &syn::Ident,
    public_struct_name: &Type,
    public_fields: &[FieldInfo],
    private_struct_name: &Type,
    private_fields: &[FieldInfo],
) -> proc_macro2::TokenStream {
    match (public_fields.is_empty(), private_fields.is_empty()) {
        (true, false) => {
            let field_assignments = private_fields.iter().map(|(name, _, _)| {
                quote! { #name: private_data.#name }
            });
            quote! {
                fn from_parts(parts: crate::codec::MessageParts) -> Result<Self, crate::DecodeError> {
                    match parts {
                        crate::codec::MessageParts::PrivateOnly(private) => {
                            let private_data: #private_struct_name = bincode::decode_from_slice(&private, bincode::config::standard())?.0;
                            Ok(Self { #(#field_assignments),* })
                        },
                        _ => Err(crate::DecodeError::InvalidMessageFormat),
                    }
                }
            }
        }
        (false, true) => {
            let field_assignments = public_fields.iter().map(|(name, _, _)| {
                quote! { #name: public_data.#name }
            });
            quote! {
                fn from_parts(parts: crate::codec::MessageParts) -> Result<Self, crate::DecodeError> {
                    match parts {
                        crate::codec::MessageParts::PublicOnly(public) => {
                            let public_data: #public_struct_name = bincode::decode_from_slice(&public, bincode::config::standard())?.0;
                            Ok(Self { #(#field_assignments),* })
                        },
                        _ => Err(crate::DecodeError::InvalidMessageFormat),
                    }
                }
            }
        }
        (false, false) => {
            let pub_assigns = public_fields.iter().map(|(name, _, _)| {
                quote! { #name: public_data.#name }
            });
            let priv_assigns = private_fields.iter().map(|(name, _, _)| {
                quote! { #name: private_data.#name }
            });
            quote! {
                fn from_parts(parts: crate::codec::MessageParts) -> Result<Self, crate::DecodeError> {
                    match parts {
                        crate::codec::MessageParts::Both { public, private } => {
                            let public_data: #public_struct_name = bincode::decode_from_slice(&public, bincode::config::standard())?.0;
                            let private_data: #private_struct_name = bincode::decode_from_slice(&private, bincode::config::standard())?.0;
                            Ok(Self { #(#pub_assigns,)* #(#priv_assigns,)* })
                        },
                        _ => Err(crate::DecodeError::InvalidMessageFormat),
                    }
                }
            }
        }
        (true, true) => unreachable!(),
    }
}
