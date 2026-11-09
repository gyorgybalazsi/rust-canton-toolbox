use crate::lf_protobuf::com::daml::daml_lf_2::{
    BuiltinType, InternedDottedName, Package, Type, self_or_imported_package_id, r#type::Sum,
};
use crate::package::ParsedDar;
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;

#[derive(Debug, Clone)]
pub struct ResolvedType {
    pub module_path: Vec<String>,
    pub type_name: String,
    pub type_args: Vec<ResolvedType>,
    pub boxed: bool,
}

impl ResolvedType {
    pub fn simple(type_name: &str) -> Self {
        ResolvedType {
            module_path: vec![],
            type_name: type_name.to_string(),
            type_args: vec![],
            boxed: false,
        }
    }

    pub fn to_token_stream(&self, current_module: &[String]) -> TokenStream {
        let inner = self.inner_token_stream(current_module);
        if self.boxed {
            quote!(Box<#inner>)
        } else {
            inner
        }
    }

    fn inner_token_stream(&self, current_module: &[String]) -> TokenStream {
        let base_path = self.relative_rust_path(current_module);
        if self.type_args.is_empty() {
            base_path
        } else {
            let args: Vec<TokenStream> = self
                .type_args
                .iter()
                .map(|a| a.to_token_stream(current_module))
                .collect();
            quote!(#base_path<#(#args),*>)
        }
    }

    fn relative_rust_path(&self, current_module: &[String]) -> TokenStream {
        let type_ident = sanitize_ident(&self.type_name);

        if self.module_path.is_empty() {
            return quote!(#type_ident);
        }

        if self.module_path == current_module {
            return quote!(#type_ident);
        }

        let common_prefix_len = self
            .module_path
            .iter()
            .zip(current_module.iter())
            .take_while(|(a, b)| a == b)
            .count();

        let super_count = current_module.len() - common_prefix_len;
        let descending_segments: Vec<Ident> = self.module_path[common_prefix_len..]
            .iter()
            .map(|s| sanitize_ident(&s.to_lowercase()))
            .collect();

        let mut path_tokens = TokenStream::new();

        for _ in 0..super_count {
            path_tokens.extend(quote!(super::));
        }

        for seg in &descending_segments {
            path_tokens.extend(quote!(#seg::));
        }

        path_tokens.extend(quote!(#type_ident));
        path_tokens
    }
}

pub fn sanitize_ident(name: &str) -> Ident {
    let mut s = name.replace(|c: char| !c.is_ascii_alphanumeric(), "_");
    if !s
        .chars()
        .next()
        .map(|c| c.is_ascii_alphabetic() || c == '_')
        .unwrap_or(false)
    {
        s = format!("_{}", s);
    }
    Ident::new(&s, Span::call_site())
}

pub fn sanitize_ident_str(name: &str) -> String {
    let mut s = name.replace(|c: char| !c.is_ascii_alphanumeric(), "_");
    if !s
        .chars()
        .next()
        .map(|c| c.is_ascii_alphabetic() || c == '_')
        .unwrap_or(false)
    {
        s = format!("_{}", s);
    }
    s
}

pub fn to_snake_case(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(c.to_lowercase().next().unwrap_or(c));
    }
    result
}

pub struct TypeResolutionContext<'a> {
    pub current_package_name: &'a str,
    pub parsed_dar: &'a ParsedDar,
    pub interned_types: &'a [Type],
    pub interned_strings: &'a [String],
    pub interned_dotted_names: &'a [InternedDottedName],
}

const NON_GENERIC_BUILTINS: &[&str] = &[
    "DamlUnit",
    "DamlBool",
    "DamlInt",
    "DamlDate",
    "DamlTimestamp",
    "DamlDecimal",
    "DamlParty",
    "DamlText",
    "DamlContractId",
    "UnsupportedBuiltin",
];

struct BuiltinMapping {
    rust_name: &'static str,
    accepts_type_args: bool,
}

fn map_builtin(builtin_type: BuiltinType) -> BuiltinMapping {
    match builtin_type {
        BuiltinType::Unit => BuiltinMapping {
            rust_name: "DamlUnit",
            accepts_type_args: false,
        },
        BuiltinType::Bool => BuiltinMapping {
            rust_name: "DamlBool",
            accepts_type_args: false,
        },
        BuiltinType::Int64 => BuiltinMapping {
            rust_name: "DamlInt",
            accepts_type_args: false,
        },
        BuiltinType::Date => BuiltinMapping {
            rust_name: "DamlDate",
            accepts_type_args: false,
        },
        BuiltinType::Timestamp => BuiltinMapping {
            rust_name: "DamlTimestamp",
            accepts_type_args: false,
        },
        BuiltinType::Numeric => BuiltinMapping {
            rust_name: "DamlDecimal",
            accepts_type_args: false,
        },
        BuiltinType::Party => BuiltinMapping {
            rust_name: "DamlParty",
            accepts_type_args: false,
        },
        BuiltinType::Text => BuiltinMapping {
            rust_name: "DamlText",
            accepts_type_args: false,
        },
        BuiltinType::ContractId => BuiltinMapping {
            rust_name: "DamlContractId",
            accepts_type_args: false,
        },
        BuiltinType::Optional => BuiltinMapping {
            rust_name: "DamlOptional",
            accepts_type_args: true,
        },
        BuiltinType::List => BuiltinMapping {
            rust_name: "DamlList",
            accepts_type_args: true,
        },
        BuiltinType::Textmap => BuiltinMapping {
            rust_name: "DamlTextMap",
            accepts_type_args: true,
        },
        BuiltinType::Genmap => BuiltinMapping {
            rust_name: "DamlMap",
            accepts_type_args: true,
        },
        _ => BuiltinMapping {
            rust_name: "UnsupportedBuiltin",
            accepts_type_args: false,
        },
    }
}

fn map_sdk_synonym(name: &str) -> Option<&'static str> {
    match name {
        "Decimal" => Some("DamlDecimal"),
        "Time" | "RelTime" => Some("DamlTimestamp"),
        _ => None,
    }
}

fn resolve_dotted_name_from_index(
    index: i32,
    interned_dotted_names: &[InternedDottedName],
    interned_strings: &[String],
) -> Vec<String> {
    interned_dotted_names
        .get(index as usize)
        .map(|dn| {
            dn.segments_interned_str
                .iter()
                .filter_map(|&idx| interned_strings.get(idx as usize).cloned())
                .collect()
        })
        .unwrap_or_default()
}

fn resolve_package_name_for_module_id(
    module_id: &crate::lf_protobuf::com::daml::daml_lf_2::ModuleId,
    ctx: &TypeResolutionContext,
) -> String {
    match &module_id.package_id {
        Some(pkg_id) => match &pkg_id.sum {
            Some(self_or_imported_package_id::Sum::SelfPackageId(_)) => {
                ctx.current_package_name.to_string()
            }
            Some(self_or_imported_package_id::Sum::ImportedPackageIdInternedStr(idx)) => {
                let hash = ctx
                    .interned_strings
                    .get(*idx as usize)
                    .cloned()
                    .unwrap_or_default();
                if let Some(pkg) = ctx.parsed_dar.packages.get(&hash) {
                    package_name_from_package(pkg)
                        .map(|n| sanitize_ident_str(&n))
                        .unwrap_or_else(|| sanitize_ident_str(&hash))
                } else {
                    sanitize_ident_str(&hash)
                }
            }
            Some(self_or_imported_package_id::Sum::PackageImportId(_)) => {
                // New-style package import reference — treat as current package for now
                ctx.current_package_name.to_string()
            }
            None => ctx.current_package_name.to_string(),
        },
        None => ctx.current_package_name.to_string(),
    }
}

pub fn package_name_from_package(package: &Package) -> Option<String> {
    let metadata = package.metadata.as_ref()?;
    package
        .interned_strings
        .get(metadata.name_interned_str as usize)
        .cloned()
}

pub fn resolve_type(typ: &Type, ctx: &TypeResolutionContext) -> ResolvedType {
    if typ.sum.is_none() {
        // Prost proto3 limitation: scalar types in oneofs with value 0 decode as None.
        // Type.Sum has `int32 interned_type = 8` and `sint64 nat = 6`.
        // A None sum is virtually always InternedType(0) — resolve it.
        return if let Some(interned_type_0) = ctx.interned_types.first() {
            resolve_type(interned_type_0, ctx)
        } else {
            ResolvedType::simple("_UnresolvedType")
        };
    }
    // Handle TApp: type application (lhs applied to rhs), e.g. `Optional Text`, `Tree a`
    // TApp replaces the args field on Con/Builtin/Syn in newer SDK versions.
    // We resolve lhs, then append rhs as a type arg (curried application).
    if let Some(Sum::Tapp(tapp)) = &typ.sum {
        let lhs = tapp
            .lhs
            .as_ref()
            .map(|t| resolve_type(t, ctx))
            .unwrap_or_else(|| ResolvedType::simple("_UnknownLhs"));
        let rhs = tapp
            .rhs
            .as_ref()
            .map(|t| resolve_type(t, ctx))
            .unwrap_or_else(|| ResolvedType::simple("_UnknownRhs"));

        let mut result = lhs;
        // Only add type args for types that accept them.
        // Non-generic builtins (DamlContractId, DamlDecimal, etc.) drop their type args.
        let is_non_generic_builtin = result.module_path.is_empty()
            && NON_GENERIC_BUILTINS.contains(&result.type_name.as_str());
        if !is_non_generic_builtin {
            result.type_args.push(rhs);
        }
        return result;
    }
    match &typ.sum {
        Some(Sum::InternedType(idx)) => {
            if let Some(interned_type) = ctx.interned_types.get(*idx as usize) {
                resolve_type(interned_type, ctx)
            } else {
                ResolvedType::simple("_InvalidInternedType")
            }
        }
        Some(Sum::Con(con)) => {
            if let Some(tycon_id) = &con.tycon {
                let type_name_segments = resolve_dotted_name_from_index(
                    tycon_id.name_interned_dname,
                    ctx.interned_dotted_names,
                    ctx.interned_strings,
                );
                let type_name = type_name_segments
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "_UnknownType".to_string());

                let mut module_path = Vec::new();
                if let Some(module_id) = &tycon_id.module {
                    let pkg_name = resolve_package_name_for_module_id(module_id, ctx);
                    module_path.push(pkg_name);
                    let mod_segments = resolve_dotted_name_from_index(
                        module_id.module_name_interned_dname,
                        ctx.interned_dotted_names,
                        ctx.interned_strings,
                    );
                    module_path.extend(mod_segments);
                }

                let type_args = con.args.iter().map(|arg| resolve_type(arg, ctx)).collect();

                ResolvedType {
                    module_path,
                    type_name,
                    type_args,
                    boxed: false,
                }
            } else {
                ResolvedType::simple("_UnknownCon")
            }
        }
        Some(Sum::Builtin(builtin)) => {
            let bt = std::convert::TryFrom::try_from(builtin.builtin).unwrap_or(BuiltinType::Unit);
            let mapping = map_builtin(bt);

            let type_args = if mapping.accepts_type_args {
                builtin
                    .args
                    .iter()
                    .map(|arg| resolve_type(arg, ctx))
                    .collect()
            } else {
                vec![]
            };

            ResolvedType {
                module_path: vec![],
                type_name: mapping.rust_name.to_string(),
                type_args,
                boxed: false,
            }
        }
        Some(Sum::Syn(syn)) => {
            if let Some(tysyn_id) = &syn.tysyn {
                let name_segments = resolve_dotted_name_from_index(
                    tysyn_id.name_interned_dname,
                    ctx.interned_dotted_names,
                    ctx.interned_strings,
                );
                let type_name = name_segments
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "_UnknownSyn".to_string());

                // Check if this synonym is from an SDK package — if so,
                // try to map it to a known daml-type-rep type.
                // Common case: "Decimal" is a synonym for "Numeric 10" in daml-stdlib.
                if let Some(mapped) = map_sdk_synonym(&type_name) {
                    return ResolvedType {
                        module_path: vec![],
                        type_name: mapped.to_string(),
                        type_args: syn.args.iter().map(|arg| resolve_type(arg, ctx)).collect(),
                        boxed: false,
                    };
                }

                let mut module_path = Vec::new();
                if let Some(module_id) = &tysyn_id.module {
                    let pkg_name = resolve_package_name_for_module_id(module_id, ctx);
                    module_path.push(pkg_name);
                    let mod_segments = resolve_dotted_name_from_index(
                        module_id.module_name_interned_dname,
                        ctx.interned_dotted_names,
                        ctx.interned_strings,
                    );
                    module_path.extend(mod_segments);
                }

                let type_args = syn.args.iter().map(|arg| resolve_type(arg, ctx)).collect();

                ResolvedType {
                    module_path,
                    type_name,
                    type_args,
                    boxed: false,
                }
            } else {
                ResolvedType::simple("_UnknownSyn")
            }
        }
        Some(Sum::Var(var)) => {
            let name = ctx
                .interned_strings
                .get(var.var_interned_str as usize)
                .cloned()
                .unwrap_or_else(|| "_UnknownVar".to_string());
            ResolvedType {
                module_path: vec![],
                type_name: name.to_uppercase(),
                type_args: vec![],
                boxed: false,
            }
        }
        Some(Sum::Forall(forall)) => {
            if let Some(body) = &forall.body {
                resolve_type(body, ctx)
            } else {
                ResolvedType::simple("_EmptyForall")
            }
        }
        Some(Sum::Nat(n)) => ResolvedType::simple(&format!("{}", n)),
        Some(Sum::Struct(_)) => ResolvedType::simple("UnsupportedAnonymousStruct"),
        _ => ResolvedType::simple("_UnknownType"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::parse_dar;

    #[test]
    fn test_resolve_type_with_context() {
        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let dar_path = std::path::PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-nested-test")
            .join("main")
            .join(".daml")
            .join("dist")
            .join("daml-nested-test-0.0.1.dar");
        let dar_path = dar_path.to_str().expect("DAR path is not valid UTF-8");
        let parsed_dar = parse_dar(dar_path).expect("Failed to parse DAR");
        let package = parsed_dar
            .packages
            .get(&parsed_dar.main_package_id)
            .expect("Main package not found");

        let ctx = TypeResolutionContext {
            current_package_name: "daml_nested_test",
            parsed_dar: &parsed_dar,
            interned_types: &package.interned_types,
            interned_strings: &package.interned_strings,
            interned_dotted_names: &package.interned_dotted_names,
        };

        // Spot-check a few types resolve correctly
        for (i, typ) in package.interned_types.iter().enumerate().take(20) {
            let resolved = resolve_type(typ, &ctx);
            if resolved.type_name != "DamlText" && resolved.type_name != "UnsupportedBuiltin" {
                println!(
                    "interned_types[{}] -> {} (path: {:?}, args: {})",
                    i,
                    resolved.type_name,
                    resolved.module_path,
                    resolved.type_args.len()
                );
            }
        }
    }
}
