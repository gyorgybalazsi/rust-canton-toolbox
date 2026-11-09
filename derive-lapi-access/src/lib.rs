use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, FieldsNamed, FieldsUnnamed, parse_macro_input};

#[proc_macro_derive(ToCreateArguments)]
pub fn derive_to_create_arguments(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    impl_to_create_arguments(&ast)
}

// TODO make module name parameter
// OR omit?
fn impl_to_create_arguments(ast: &DeriveInput) -> TokenStream {
    let name = &ast.ident;

    let fields = match &ast.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields_named) => &fields_named.named,
            _ => panic!("ToUpdateInput  only supports named fields"),
        },
        _ => panic!("ToUpdateInput  only supports structs"),
    };

    let field_names: Vec<_> = fields.iter().map(|f| &f.ident).collect();
    let field_labels: Vec<_> = fields
        .iter()
        .map(|f| {
            let ident = &f.ident;
            ident.as_ref().unwrap().to_string()
        })
        .collect();

    let generated = quote! {
        impl ToCreateArguments  for #name {
            fn to_create_arguments(&self) -> Record {
                let mut fields = vec![];
                #(
                    fields.push(self.#field_names.to_lapi_record_field(#field_labels));
                )*
                Record {
                    record_id: None,
                    fields,
                }
            }
        }
    };
    generated.into()
}

fn snake_to_camel(s: &str) -> String {
    let mut result = String::new();
    let mut uppercase_next = false;
    for c in s.chars() {
        if c == '_' {
            uppercase_next = true;
        } else if uppercase_next {
            result.push(c.to_ascii_uppercase());
            uppercase_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

#[proc_macro_derive(LapiAccess)]
pub fn derive_lapi_access(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    impl_lapi_access(&ast)
}

fn impl_lapi_access(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    match &ast.data {
        Data::Enum(data_enum) => {
            let mut match_arms = Vec::new();
            for variant in &data_enum.variants {
                let v_ident = &variant.ident;
                match &variant.fields {
                    Fields::Unit => {
                        match_arms.push(quote! {
                            #name::#v_ident => {
                                ledger_api::v2::Value {
                                    sum: Some(ledger_api::v2::value::Sum::Enum(ledger_api::v2::Enum {
                                        enum_id: None,
                                        constructor: stringify!(#v_ident).to_string(),
                                    }))
                                }
                            }
                        });
                    }
                    Fields::Named(FieldsNamed { named, .. }) => {
                        let field_idents: Vec<_> =
                            named.iter().map(|f| f.ident.as_ref().unwrap()).collect();
                        let field_labels: Vec<_> = field_idents
                            .iter()
                            .map(|id| snake_to_camel(&id.to_string()))
                            .collect();
                        match_arms.push(quote! {
                            #name::#v_ident { #( #field_idents ),* } => {
                                ledger_api::v2::Value {
                                    sum: Some(ledger_api::v2::value::Sum::Variant(Box::new(ledger_api::v2::Variant {
                                        variant_id: None,
                                        constructor: stringify!(#v_ident).to_string(),
                                        value: Some(Box::new(ledger_api::v2::Value {
                                            sum: Some(ledger_api::v2::value::Sum::Record(ledger_api::v2::Record {
                                                record_id: None,
                                                fields: vec![
                                                    #(
                                                        ledger_api::v2::RecordField {
                                                            label: #field_labels.to_string(),
                                                            value: Some(#field_idents.to_lapi_value()),
                                                        }
                                                    ),*
                                                ],
                                            })),
                                        })),
                                    })))
                                }
                            }
                        });
                    }
                    Fields::Unnamed(FieldsUnnamed { .. }) => {
                        panic!("LapiAccess does not support tuple variants")
                    }
                }
            }
            let mut from_match_arms = Vec::new();
            for variant in &data_enum.variants {
                let v_ident = &variant.ident;
                match &variant.fields {
                    Fields::Unit => {
                        from_match_arms.push(quote! {
                            (stringify!(#v_ident), None) => Some(#name::#v_ident),
                        });
                    }
                    Fields::Named(FieldsNamed { named, .. }) => {
                        let field_idents: Vec<_> =
                            named.iter().map(|f| f.ident.as_ref().unwrap()).collect();
                        let field_types: Vec<_> = named.iter().map(|f| &f.ty).collect();
                        let field_labels: Vec<_> = field_idents
                            .iter()
                            .map(|id| snake_to_camel(&id.to_string()))
                            .collect::<Vec<_>>();
                        from_match_arms.push(quote! {
                                (stringify!(#v_ident), Some(ref boxed_val)) => {
                                    if let ledger_api::v2::Value { sum: Some(ledger_api::v2::value::Sum::Record(rec)), .. } = &**boxed_val {
                                        Some(#name::#v_ident {
                                            #(
                                                #field_idents: {
                                                    let field = rec.fields.iter().find(|f| f.label == #field_labels)?;
                                                    <#field_types as LapiAccess>::from_lapi_value(field.value.as_ref()?)?
                                                }
                                            ),*
                                        })
                                    } else {
                                        None
                                    }
                                },
                            });
                    }
                    Fields::Unnamed(FieldsUnnamed { .. }) => {
                        panic!("LapiAccess does not support tuple variants")
                    }
                }
            }
            let expanded = quote! {
                impl LapiAccess for #name {
                    fn to_lapi_value(&self) -> ledger_api::v2::Value {
                        match self {
                            #(#match_arms),*
                        }
                    }

                    fn from_lapi_value(value: &ledger_api::v2::Value) -> Option<Self> {
                        match value.sum.as_ref()? {
                            ledger_api::v2::value::Sum::Enum(e) => {
                                match (e.constructor.as_str(), None as Option<&ledger_api::v2::Value>) {
                                    #(#from_match_arms)*
                                    _ => None
                                }
                            },
                            ledger_api::v2::value::Sum::Variant(var) => {
                                match (var.constructor.as_str(), var.value.as_deref()) {
                                    #(#from_match_arms)*
                                    _ => None
                                }
                            },
                            _ => None
                        }
                    }
                }
            };
            expanded.into()
        }
        Data::Struct(data_struct) => {
            let fields = match &data_struct.fields {
                Fields::Named(fields_named) => &fields_named.named,
                _ => panic!("LapiAccess only supports named fields for structs"),
            };
            let field_names: Vec<_> = fields.iter().map(|f| &f.ident).collect();
            let field_types: Vec<_> = fields.iter().map(|f| &f.ty).collect();
            let field_labels: Vec<_> = fields
                .iter()
                .map(|f| {
                    let ident = &f.ident;
                    let snake = ident.as_ref().unwrap().to_string();
                    snake_to_camel(&snake)
                })
                .collect::<Vec<_>>();
            let expanded = quote! {
                impl LapiAccess for #name {
                    fn to_lapi_value(&self) -> ledger_api::v2::Value {
                        let mut fields = vec![];
                        #(
                            fields.push(self.#field_names.to_lapi_record_field(#field_labels));
                        )*
                        ledger_api::v2::Value {
                            sum: Some(ledger_api::v2::value::Sum::Record(Record {
                                record_id: None,
                                fields,
                            })),
                        }
                    }

                    fn from_lapi_value(value: &ledger_api::v2::Value) -> Option<Self> {
                        if let ledger_api::v2::Value { sum: Some(ledger_api::v2::value::Sum::Record(rec)), .. } = value {
                            Some(Self {
                                #(
                                    #field_names: {
                                        let field = rec.fields.iter().find(|f| f.label == #field_labels)?;
                                        <#field_types as LapiAccess>::from_lapi_value(field.value.as_ref()?)?
                                    }
                                ),*
                            })
                        } else {
                            None
                        }
                    }
                }
            };
            expanded.into()
        }
        _ => panic!("LapiAccess can only be derived for enums or structs with named fields"),
    }
}

// Test to understand the difference between field names and labels
#[cfg(test)]
#[test]
fn test() {
    let ts = quote! {
        struct Point {
            x: i32,
            y: i32,
        }
    };
    let ast: DeriveInput = syn::parse2(ts).unwrap();

    let fields = match &ast.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields_named) => &fields_named.named,
            _ => panic!("ToUpdateInput  only supports named fields"),
        },
        _ => panic!("ToUpdateInput  only supports structs"),
    };

    let field_names: Vec<_> = fields.iter().map(|f| &f.ident).collect();
    let field_labels: Vec<_> = fields
        .iter()
        .map(|f| {
            let ident = &f.ident;
            ident.as_ref().unwrap().to_string()
        })
        .collect();
    dbg!(field_names);
    dbg!(field_labels);
}
