use proc_macro::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Fields, Meta, MetaNameValue, Type, parse_macro_input};

#[proc_macro_derive(AeadMessage, attributes(message_id, Aead, AeadSerialisation))]
pub fn derive_aead_message(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let message_id = extract_message_id(&input.attrs);
    let name = &input.ident;
    let fields = extract_struct_fields(&input.data);
    let fields = categorize_fields(fields);

    let public_struct_name = if fields.public_fields.is_empty() {
        syn::parse_str::<syn::Type>("()").unwrap()
    } else {
        let struct_name = syn::Ident::new(&format!("{name}AssociatedData"), name.span());
        syn::Type::Path(syn::TypePath {
            qself: None,
            path: struct_name.into(),
        })
    };

    let secret_struct_name = if fields.secret_fields.is_empty() {
        syn::parse_str::<syn::Type>("()").unwrap()
    } else {
        let struct_name = syn::Ident::new(&format!("{name}EncryptedData"), name.span());
        syn::Type::Path(syn::TypePath {
            qself: None,
            path: struct_name.into(),
        })
    };

    let public_struct = generate_public_struct(&public_struct_name, &fields.public_fields);
    let secret_struct = generate_secret_struct(&secret_struct_name, &fields.secret_fields);

    let nonce_impl = generate_nonce_impl(&fields.nonce_field);
    let public_bytes_impl = generate_public_bytes_impl(&public_struct_name, &fields.public_fields);
    let secret_bytes_impl = generate_secret_bytes_impl(&secret_struct_name, &fields.secret_fields);

    let from_parts_impl = generate_from_parts_impl(name, &fields);

    let expanded = quote! {
        #public_struct
        #secret_struct

        impl crate::codec::Message for #name {
            type AssociatedData = #public_struct_name;
            const MESSAGE_ID: u8 = #message_id as u8;
            #nonce_impl
            #public_bytes_impl
            #secret_bytes_impl
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

type FieldInfo = (syn::Ident, syn::Type, Vec<Attribute>);
struct FieldClassification {
    public_fields: Vec<FieldInfo>,
    secret_fields: Vec<FieldInfo>,
    nonce_field: Option<FieldInfo>,
}

fn categorize_fields(fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>) -> FieldClassification {
    let mut public_fields = Vec::new();
    let mut secret_fields = Vec::new();
    let mut nonce_field = None;

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_type = field.ty.clone();

        let mut is_associated_data = false;
        let mut is_encrypted = false;
        let mut is_nonce = false;

        for attr in &field.attrs {
            if attr.path().is_ident("Aead") {
                match &attr.meta {
                    Meta::List(list) => {
                        let tokens_str = list.tokens.to_string();
                        if tokens_str == "associated_data" {
                            is_associated_data = true;
                        } else if tokens_str == "encrypted" {
                            is_encrypted = true;
                        } else if tokens_str == "Nonce" {
                            is_nonce = true;
                        } else {
                            panic!(
                                "Unknown Aead attribute option '{tokens_str}' for field {field_name}. Valid options are: associated_data, encrypted, Nonce"
                            );
                        }
                    }
                    _ => panic!("Aead attribute must be used as #[Aead(option)] for field {field_name}"),
                }
            }
        }

        let count = [is_associated_data, is_encrypted, is_nonce]
            .iter()
            .filter(|&&x| x)
            .count();
        if count > 1 {
            panic!("Field {field_name} cannot have multiple Aead attributes");
        } else if count < 1 {
            panic!(
                "Field {field_name} must be marked as either #[Aead(associated_data)], #[Aead(encrypted)], or #[Aead(Nonce)]"
            )
        }

        if is_associated_data {
            public_fields.push((field_name.clone(), field_type.clone(), field.attrs.clone()));
        }

        if is_encrypted {
            secret_fields.push((field_name.clone(), field_type.clone(), field.attrs.clone()));
        }

        if is_nonce {
            nonce_field = Some((field_name.clone(), field_type.clone(), field.attrs.clone()));
        }
    }

    if public_fields.is_empty() && secret_fields.is_empty() {
        panic!("Message must have at least one field marked as associated_data or encrypted");
    }

    FieldClassification {
        public_fields,
        secret_fields,
        nonce_field,
    }
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

fn generate_secret_struct(secret_struct_name: &Type, secret_fields: &[FieldInfo]) -> proc_macro2::TokenStream {
    if secret_fields.is_empty() {
        return quote! {};
    }

    let secret_field_defs = secret_fields.iter().map(|(name, ty, attrs)| {
        let passthrough_attrs = extract_passthrough_attributes(attrs);
        quote! { #(#passthrough_attrs)* pub #name: #ty }
    });

    quote! {
        #[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
        pub(crate) struct #secret_struct_name {
            #(#secret_field_defs),*
        }
    }
}

fn generate_nonce_impl(nonce_field: &Option<FieldInfo>) -> proc_macro2::TokenStream {
    if let Some((nonce_name, nonce_type, _)) = nonce_field {
        // Generate specific implementations for known types
        if let syn::Type::Path(type_path) = nonce_type
            && let Some(ident) = type_path.path.get_ident()
        {
            if ident == "u64" {
                return quote! {
                    fn with_nonce_bytes<F, R>(&self, f: F) -> Result<bool, crate::EncodeError>
                    where
                        F: FnOnce(&[u8]) -> Result<R, crate::EncodeError>,
                    {
                        let nonce_bytes = self.#nonce_name.to_le_bytes();
                        f(&nonce_bytes)?;
                        Ok(true)
                    }
                };
            } else if ident == "u32" {
                return quote! {
                    fn with_nonce_bytes<F, R>(&self, f: F) -> Result<bool, crate::EncodeError>
                    where
                        F: FnOnce(&[u8]) -> Result<R, crate::EncodeError>,
                    {
                        let nonce_bytes = self.#nonce_name.to_le_bytes();
                        f(&nonce_bytes)?;
                        Ok(true)
                    }
                };
            }
        }

        // Fallback for other types using the Nonceable trait
        quote! {
            fn with_nonce_bytes<F, R>(&self, f: F) -> Result<bool, crate::EncodeError>
            where
                F: FnOnce(&[u8]) -> Result<R, crate::EncodeError>,
            {
                use crate::codec::Nonceable;
                let nonce_bytes = self.#nonce_name.as_nonce_bytes();
                f(nonce_bytes.as_ref())?;
                Ok(true)
            }
        }
    } else {
        quote! {
            fn with_nonce_bytes<F, R>(&self, _f: F) -> Result<bool, crate::EncodeError>
            where
                F: FnOnce(&[u8]) -> Result<R, crate::EncodeError>,
            {
                // No custom nonce, so don't call the function and return false
                Ok(false)
            }
        }
    }
}

fn generate_public_bytes_impl(public_struct_name: &Type, public_fields: &[FieldInfo]) -> proc_macro2::TokenStream {
    let public_data = if !public_fields.is_empty() {
        let field_assignments = public_fields.iter().map(|(name, _, _)| {
            quote! { #name: self.#name.clone() }
        });
        quote! {
            let public_data = #public_struct_name { #(#field_assignments),* };
            let public_bytes = bincode::encode_to_vec(&public_data, crate::BINCODE_CONFIG)?;
        }
    } else {
        quote! { let public_bytes : Vec<u8> = Vec::new(); }
    };

    quote! {
        fn public_bytes(&self) -> Result<Vec<u8>, crate::EncodeError> {
            #public_data
            Ok(public_bytes)
        }
    }
}

fn generate_secret_bytes_impl(secret_struct_name: &Type, secret_fields: &[FieldInfo]) -> proc_macro2::TokenStream {
    let secret_data = if !secret_fields.is_empty() {
        let field_assignments = secret_fields.iter().map(|(name, _, _)| {
            quote! { #name: self.#name.clone() }
        });
        quote! {
            let secret_data = #secret_struct_name { #(#field_assignments),* };
            let secret_bytes = bincode::encode_to_vec(&secret_data, crate::BINCODE_CONFIG)?;
        }
    } else {
        quote! { let secret_bytes : Vec<u8> = Vec::new(); }
    };

    quote! {
        fn secret_bytes(&self) -> Result<Vec<u8>, crate::EncodeError> {
            #secret_data
            Ok(secret_bytes)
        }
    }
}

fn generate_from_parts_impl(name: &syn::Ident, fields: &FieldClassification) -> proc_macro2::TokenStream {
    let public_decode = if !fields.public_fields.is_empty() {
        let public_struct_name = syn::Ident::new(&format!("{name}AssociatedData"), name.span());
        quote! {
            let public_data: #public_struct_name = {
                let (decoded, _): (#public_struct_name, usize) = bincode::decode_from_slice(public_bytes, crate::BINCODE_CONFIG).unwrap();
                decoded
            };
        }
    } else {
        quote! {}
    };

    let secret_decode = if !fields.secret_fields.is_empty() {
        let secret_struct_name = syn::Ident::new(&format!("{name}EncryptedData"), name.span());
        quote! {
            let secret_data: #secret_struct_name = {
                let (decoded, _): (#secret_struct_name, usize) = bincode::decode_from_slice(secret_bytes, crate::BINCODE_CONFIG).unwrap();
                decoded
            };
        }
    } else {
        quote! {}
    };

    let field_assignments = fields
        .public_fields
        .iter()
        .chain(fields.secret_fields.iter())
        .map(|(name, _, _)| {
            if fields.public_fields.iter().any(|(pub_name, _, _)| pub_name == name) {
                quote! { #name: public_data.#name }
            } else {
                quote! { #name: secret_data.#name }
            }
        });

    let nonce_assignment = if let Some((nonce_name, nonce_type, _)) = &fields.nonce_field {
        // Generate code to extract the nonce value from the nonce bytes
        if let syn::Type::Path(type_path) = nonce_type {
            if let Some(ident) = type_path.path.get_ident() {
                if ident == "u64" {
                    quote! {
                        #nonce_name: {
                            let mut bytes = [0u8; 8];
                            bytes.copy_from_slice(&_nonce[..8]);
                            u64::from_le_bytes(bytes)
                        },
                    }
                } else if ident == "u32" {
                    quote! {
                        #nonce_name: {
                            let mut bytes = [0u8; 4];
                            bytes.copy_from_slice(&_nonce[..4]);
                            u32::from_le_bytes(bytes)
                        },
                    }
                } else {
                    // Fallback for other types using the Nonceable trait
                    quote! {
                        #nonce_name: {
                            use crate::codec::Nonceable;
                            let mut bytes = [0u8; std::mem::size_of::<#nonce_type>()];
                            let len = bytes.len().min(_nonce.len());
                            bytes[..len].copy_from_slice(&_nonce[..len]);
                            <#nonce_type as crate::codec::Nonceable>::from_nonce_bytes(bytes)
                        },
                    }
                }
            } else {
                // Fallback for complex types
                quote! {
                    #nonce_name: {
                        use crate::codec::Nonceable;
                        let mut bytes = [0u8; std::mem::size_of::<#nonce_type>()];
                        let len = bytes.len().min(_nonce.len());
                        bytes[..len].copy_from_slice(&_nonce[..len]);
                        <#nonce_type as crate::codec::Nonceable>::from_nonce_bytes(bytes)
                    },
                }
            }
        } else {
            // Fallback for non-path types
            quote! {
                #nonce_name: {
                    use crate::codec::Nonceable;
                    let mut bytes = [0u8; std::mem::size_of::<#nonce_type>()];
                    let len = bytes.len().min(_nonce.len());
                    bytes[..len].copy_from_slice(&_nonce[..len]);
                    <#nonce_type as crate::codec::Nonceable>::from_nonce_bytes(bytes)
                },
            }
        }
    } else {
        quote! {}
    };

    quote! {
        fn from_parts(_nonce: &[u8; crate::codec::NONCE_SIZE], public_bytes: &[u8], secret_bytes: &[u8]) -> Self {
            #public_decode
            #secret_decode
            Self {
                #(#field_assignments,)*
                #nonce_assignment
            }
        }
    }
}
