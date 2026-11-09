use crate::lf_protobuf::com::daml::daml_lf_2::{
    DefDataType, DefTypeSyn, Module, Package, def_data_type::DataCons,
};
use crate::resolve_type::{ResolvedType, TypeResolutionContext, resolve_type};
use anyhow::{Context, Result, bail};

// --- Helpers ---

pub fn resolve_interned_dotted_name(index: i32, package: &Package) -> Result<Vec<String>> {
    let dotted_name = package
        .interned_dotted_names
        .get(index as usize)
        .context("interned dotted name index out of bounds")?;
    dotted_name
        .segments_interned_str
        .iter()
        .map(|&idx| {
            package
                .interned_strings
                .get(idx as usize)
                .cloned()
                .context("interned string index out of bounds")
        })
        .collect()
}

pub fn module_path(module: &Module, package: &Package) -> Result<Vec<String>> {
    resolve_interned_dotted_name(module.name_interned_dname, package)
}

pub fn data_type_name_segments(
    def_data_type: &DefDataType,
    package: &Package,
) -> Result<Vec<String>> {
    resolve_interned_dotted_name(def_data_type.name_interned_dname, package)
}

fn type_params_from_def(def_data_type: &DefDataType, package: &Package) -> Vec<String> {
    def_data_type
        .params
        .iter()
        .filter_map(|p| {
            package
                .interned_strings
                .get(p.var_interned_str as usize)
                .cloned()
        })
        .map(|s| s.to_uppercase())
        .collect()
}

fn build_module_path(pkg_name: &str, module: &Module, package: &Package) -> Result<Vec<String>> {
    let mut path = vec![pkg_name.to_string()];
    path.extend(module_path(module, package)?);
    Ok(path)
}

// --- Record ---

#[derive(Debug, Clone)]
pub struct DamlRecordRep {
    pub module_path: Vec<String>,
    pub record_name: String,
    pub type_params: Vec<String>,
    pub fields: Vec<DamlRecordFieldRep>,
    pub is_template: bool,
}

#[derive(Debug, Clone)]
pub struct DamlRecordFieldRep {
    pub field_name: String,
    pub resolved_type: ResolvedType,
}

impl DamlRecordRep {
    pub fn try_from_record(
        def_data_type: &DefDataType,
        module: &Module,
        package: &Package,
        ctx: &TypeResolutionContext,
        is_template: bool,
    ) -> Result<Self> {
        let mp = build_module_path(ctx.current_package_name, module, package)?;
        let name_segments = data_type_name_segments(def_data_type, package)?;
        let record_name = name_segments
            .last()
            .cloned()
            .context("empty data type name")?;
        let type_params = type_params_from_def(def_data_type, package);

        if let Some(DataCons::Record(record)) = &def_data_type.data_cons {
            let fields = record
                .fields
                .iter()
                .map(|field| {
                    let field_name = package
                        .interned_strings
                        .get(field.field_interned_str as usize)
                        .cloned()
                        .unwrap_or_else(|| "_invalid".to_string());
                    let resolved_type = field
                        .r#type
                        .as_ref()
                        .map(|typ| resolve_type(typ, ctx))
                        .unwrap_or_else(|| ResolvedType::simple("_UnknownType"));
                    DamlRecordFieldRep {
                        field_name,
                        resolved_type,
                    }
                })
                .collect();

            Ok(DamlRecordRep {
                module_path: mp,
                record_name,
                type_params,
                fields,
                is_template,
            })
        } else {
            bail!("DefDataType is not a record")
        }
    }
}

// --- Variant ---

#[derive(Debug, Clone)]
pub struct DamlVariantRep {
    pub module_path: Vec<String>,
    pub variant_name: String,
    pub type_params: Vec<String>,
    pub constructors: Vec<VariantConstructor>,
}

#[derive(Debug, Clone)]
pub struct VariantConstructor {
    pub name: String,
    pub payload: ResolvedType,
}

impl DamlVariantRep {
    pub fn try_from_variant(
        def_data_type: &DefDataType,
        module: &Module,
        package: &Package,
        ctx: &TypeResolutionContext,
    ) -> Result<Self> {
        let mp = build_module_path(ctx.current_package_name, module, package)?;
        let name_segments = data_type_name_segments(def_data_type, package)?;
        let variant_name = name_segments
            .last()
            .cloned()
            .context("empty data type name")?;
        let type_params = type_params_from_def(def_data_type, package);

        if let Some(DataCons::Variant(variant)) = &def_data_type.data_cons {
            let constructors = variant
                .fields
                .iter()
                .map(|field| {
                    let name = package
                        .interned_strings
                        .get(field.field_interned_str as usize)
                        .cloned()
                        .unwrap_or_else(|| "_Invalid".to_string());
                    let payload = field
                        .r#type
                        .as_ref()
                        .map(|typ| resolve_type(typ, ctx))
                        .unwrap_or_else(|| ResolvedType::simple("DamlUnit"));
                    VariantConstructor { name, payload }
                })
                .collect();

            Ok(DamlVariantRep {
                module_path: mp,
                variant_name,
                type_params,
                constructors,
            })
        } else {
            bail!("DefDataType is not a variant")
        }
    }
}

// --- Enum ---

#[derive(Debug, Clone)]
pub struct DamlEnumRep {
    pub module_path: Vec<String>,
    pub enum_name: String,
    pub constructors: Vec<String>,
}

impl DamlEnumRep {
    pub fn try_from_enum(
        def_data_type: &DefDataType,
        module: &Module,
        package: &Package,
        ctx: &TypeResolutionContext,
    ) -> Result<Self> {
        let mp = build_module_path(ctx.current_package_name, module, package)?;
        let name_segments = data_type_name_segments(def_data_type, package)?;
        let enum_name = name_segments
            .last()
            .cloned()
            .context("empty data type name")?;

        if let Some(DataCons::Enum(enum_cons)) = &def_data_type.data_cons {
            let constructors = enum_cons
                .constructors_interned_str
                .iter()
                .filter_map(|&idx| package.interned_strings.get(idx as usize).cloned())
                .collect();
            Ok(DamlEnumRep {
                module_path: mp,
                enum_name,
                constructors,
            })
        } else {
            bail!("DefDataType is not an enum")
        }
    }
}

// --- Type Alias ---

#[derive(Debug, Clone)]
pub struct DamlTypeAliasRep {
    pub module_path: Vec<String>,
    pub alias_name: String,
    pub type_params: Vec<String>,
    pub target_type: ResolvedType,
}

impl DamlTypeAliasRep {
    pub fn try_from_synonym(
        def_type_syn: &DefTypeSyn,
        module: &Module,
        package: &Package,
        ctx: &TypeResolutionContext,
    ) -> Result<Self> {
        let mp = build_module_path(ctx.current_package_name, module, package)?;
        let name_segments =
            resolve_interned_dotted_name(def_type_syn.name_interned_dname, package)?;
        let alias_name = name_segments
            .last()
            .cloned()
            .context("empty type synonym name")?;

        let type_params: Vec<String> = def_type_syn
            .params
            .iter()
            .filter_map(|p| {
                package
                    .interned_strings
                    .get(p.var_interned_str as usize)
                    .cloned()
            })
            .map(|s| s.to_uppercase())
            .collect();

        let target_type = def_type_syn
            .r#type
            .as_ref()
            .map(|typ| resolve_type(typ, ctx))
            .unwrap_or_else(|| ResolvedType::simple("_UnknownType"));

        Ok(DamlTypeAliasRep {
            module_path: mp,
            alias_name,
            type_params,
            target_type,
        })
    }
}

// --- Legacy structs (unused, kept for backward compat) ---

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TemplateRep {
    pub record: DamlRecordRep,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ChoiceRep {
    pub record: DamlRecordRep,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::{package_from_dar, parse_dar};

    #[test]
    fn test_try_convert_data_types() -> Result<()> {
        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let dar_path = std::path::PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-ticketoffer")
            .join("main")
            .join(".daml")
            .join("dist")
            .join("daml-ticketoffer-0.0.1.dar");
        let dar_path = dar_path.to_str().expect("DAR path is not valid UTF-8");
        let parsed_dar = parse_dar(dar_path)?;
        let package = package_from_dar(dar_path)?;

        let ctx = TypeResolutionContext {
            current_package_name: "daml_ticketoffer",
            parsed_dar: &parsed_dar,
            interned_types: &package.interned_types,
            interned_strings: &package.interned_strings,
            interned_dotted_names: &package.interned_dotted_names,
        };

        let module = package.modules.first().context("No modules in package")?;
        for def_data_type in &module.data_types {
            match &def_data_type.data_cons {
                Some(DataCons::Record(_)) => {
                    match DamlRecordRep::try_from_record(
                        def_data_type,
                        module,
                        &package,
                        &ctx,
                        false,
                    ) {
                        Ok(rep) => println!("Record: {:#?}", rep),
                        Err(e) => println!("Failed record: {}", e),
                    }
                }
                Some(DataCons::Variant(_)) => {
                    match DamlVariantRep::try_from_variant(def_data_type, module, &package, &ctx) {
                        Ok(rep) => println!("Variant: {:#?}", rep),
                        Err(e) => println!("Failed variant: {}", e),
                    }
                }
                Some(DataCons::Enum(_)) => {
                    match DamlEnumRep::try_from_enum(def_data_type, module, &package, &ctx) {
                        Ok(rep) => println!("Enum: {:#?}", rep),
                        Err(e) => println!("Failed enum: {}", e),
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}
