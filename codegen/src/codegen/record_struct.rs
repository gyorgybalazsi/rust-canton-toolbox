use crate::daml_custom_data_type_reps::record::{
    DamlEnumRep, DamlRecordRep, DamlTypeAliasRep, DamlVariantRep,
};
use crate::lf_protobuf::com::daml::daml_lf_2::{Package, def_data_type::DataCons};
use crate::resolve_type::{
    TypeResolutionContext, package_name_from_package, sanitize_ident, sanitize_ident_str,
    to_snake_case,
};
use anyhow::{Context, Result};
use proc_macro2::{Ident, TokenStream};
use quote::quote;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::Write;

// --- ModuleTree ---

struct ModuleTree {
    children: BTreeMap<String, ModuleTree>,
    records: Vec<DamlRecordRep>,
    variants: Vec<DamlVariantRep>,
    enums: Vec<DamlEnumRep>,
    type_aliases: Vec<DamlTypeAliasRep>,
}

impl ModuleTree {
    fn new() -> Self {
        ModuleTree {
            children: BTreeMap::new(),
            records: Vec::new(),
            variants: Vec::new(),
            enums: Vec::new(),
            type_aliases: Vec::new(),
        }
    }

    fn node_at_path(&mut self, path: &[String]) -> &mut ModuleTree {
        let mut current = self;
        for segment in path {
            current = current
                .children
                .entry(segment.clone())
                .or_insert_with(ModuleTree::new);
        }
        current
    }

    fn insert_record(&mut self, record: DamlRecordRep) {
        let path = record.module_path.clone();
        self.node_at_path(&path).records.push(record);
    }

    fn insert_variant(&mut self, variant: DamlVariantRep) {
        let path = variant.module_path.clone();
        self.node_at_path(&path).variants.push(variant);
    }

    fn insert_enum(&mut self, e: DamlEnumRep) {
        let path = e.module_path.clone();
        self.node_at_path(&path).enums.push(e);
    }

    fn insert_type_alias(&mut self, alias: DamlTypeAliasRep) {
        let path = alias.module_path.clone();
        self.node_at_path(&path).type_aliases.push(alias);
    }

    fn has_types(&self) -> bool {
        !self.records.is_empty()
            || !self.variants.is_empty()
            || !self.enums.is_empty()
            || !self.type_aliases.is_empty()
    }

    fn to_token_stream(&self, current_path: &[String]) -> TokenStream {
        let mut tokens = TokenStream::new();

        // Emit use statements if this module has types
        if self.has_types() {
            tokens.extend(quote! {
                use daml_type_rep::built_in_types::*;
                use daml_type_rep::lapi_access::LapiAccess;
                use daml_type_rep::lapi_access::ToCreateArguments;
                use derive_lapi_access::{LapiAccess, ToCreateArguments};
                use ledger_api::v2::Record;
                use ledger_api::v2::RecordField;
                use ledger_api::v2::Value;
                use ledger_api::v2::value::Sum;
            });
        }

        for record in &self.records {
            tokens.extend(generate_record_struct(record, current_path));
        }

        for variant in &self.variants {
            tokens.extend(generate_variant_enum(variant, current_path));
        }

        for e in &self.enums {
            tokens.extend(generate_simple_enum(e));
        }

        for alias in &self.type_aliases {
            tokens.extend(generate_type_alias(alias, current_path));
        }

        for (name, child) in &self.children {
            let mod_ident = sanitize_ident(&name.to_lowercase());
            let mut child_path: Vec<String> = current_path.to_vec();
            child_path.push(name.clone());
            let child_tokens = child.to_token_stream(&child_path);
            tokens.extend(quote!(
                #[allow(non_snake_case, unused_imports, dead_code)]
                pub mod #mod_ident {
                    #child_tokens
                }
            ));
        }

        tokens
    }
}

// --- Code generation functions ---

fn generate_record_struct(record: &DamlRecordRep, current_module: &[String]) -> TokenStream {
    let struct_name = sanitize_ident(&record.record_name);

    let params: Vec<Ident> = record
        .type_params
        .iter()
        .map(|p| sanitize_ident(p))
        .collect();

    let field_tokens: Vec<TokenStream> = record
        .fields
        .iter()
        .map(|f| {
            let original_name = &f.field_name;
            let snake_name = to_snake_case(original_name);
            let field_ident = sanitize_ident(&snake_name);
            let field_type = f.resolved_type.to_token_stream(current_module);

            if snake_name != *original_name {
                quote! {
                    #[serde(rename = #original_name)]
                    pub #field_ident: #field_type
                }
            } else {
                quote! {
                    pub #field_ident: #field_type
                }
            }
        })
        .collect();

    let has_generics = !params.is_empty();

    // LapiAccess derive doesn't support generic types.
    // serde::Deserialize requires Deserialize on all field types, which daml-type-rep types don't impl.
    let derives = if record.is_template && !has_generics {
        quote!(#[derive(Debug, Clone, serde::Serialize, LapiAccess, ToCreateArguments)])
    } else if !has_generics {
        quote!(#[derive(Debug, Clone, serde::Serialize, LapiAccess)])
    } else {
        quote!(#[derive(Debug, Clone, serde::Serialize)])
    };

    if params.is_empty() {
        quote! {
            #derives
            pub struct #struct_name {
                #(#field_tokens,)*
            }
        }
    } else {
        quote! {
            #derives
            pub struct #struct_name<#(#params),*> {
                #(#field_tokens,)*
            }
        }
    }
}

fn generate_variant_enum(variant: &DamlVariantRep, current_module: &[String]) -> TokenStream {
    let enum_name = sanitize_ident(&variant.variant_name);

    let params: Vec<Ident> = variant
        .type_params
        .iter()
        .map(|p| sanitize_ident(p))
        .collect();

    let constructor_tokens: Vec<TokenStream> = variant
        .constructors
        .iter()
        .map(|c| {
            let ctor_name = sanitize_ident(&c.name);
            // If payload is DamlUnit, emit a unit variant
            if c.payload.type_name == "DamlUnit"
                && c.payload.module_path.is_empty()
                && c.payload.type_args.is_empty()
            {
                quote!(#ctor_name)
            } else {
                let payload_type = c.payload.to_token_stream(current_module);
                quote!(#ctor_name(#payload_type))
            }
        })
        .collect();

    // Variant enums: LapiAccess derive doesn't support tuple variants.
    // The individual constructor payload types (records) have LapiAccess instead.
    if params.is_empty() {
        quote! {
            #[derive(Debug, Clone, serde::Serialize)]
            pub enum #enum_name {
                #(#constructor_tokens,)*
            }
        }
    } else {
        quote! {
            #[derive(Debug, Clone, serde::Serialize)]
            pub enum #enum_name<#(#params),*> {
                #(#constructor_tokens,)*
            }
        }
    }
}

fn generate_simple_enum(e: &DamlEnumRep) -> TokenStream {
    let enum_name = sanitize_ident(&e.enum_name);
    let constructors: Vec<Ident> = e.constructors.iter().map(|c| sanitize_ident(c)).collect();

    quote! {
        #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, LapiAccess)]
        pub enum #enum_name {
            #(#constructors,)*
        }
    }
}

fn generate_type_alias(alias: &DamlTypeAliasRep, current_module: &[String]) -> TokenStream {
    let alias_ident = sanitize_ident(&alias.alias_name);
    let target = alias.target_type.to_token_stream(current_module);

    let params: Vec<Ident> = alias
        .type_params
        .iter()
        .map(|p| sanitize_ident(p))
        .collect();

    if params.is_empty() {
        quote!(pub type #alias_ident = #target;)
    } else {
        quote!(pub type #alias_ident<#(#params),*> = #target;)
    }
}

// --- Recursive type detection ---

fn resolve_recursive_types(records: &mut [DamlRecordRep], variants: &mut [DamlVariantRep]) {
    // Build a map of type names → index for lookup
    let mut type_keys: HashMap<(Vec<String>, String), usize> = HashMap::new();
    let mut edges: Vec<Vec<(usize, usize)>> = Vec::new(); // (target_node, field_index)

    // Assign indices: records first, then variants
    let num_records = records.len();
    let total = num_records + variants.len();

    for (i, r) in records.iter().enumerate() {
        type_keys.insert((r.module_path.clone(), r.record_name.clone()), i);
    }
    for (i, v) in variants.iter().enumerate() {
        type_keys.insert(
            (v.module_path.clone(), v.variant_name.clone()),
            num_records + i,
        );
    }

    // Build adjacency list
    edges.resize(total, Vec::new());
    for (i, r) in records.iter().enumerate() {
        for (fi, f) in r.fields.iter().enumerate() {
            if let Some(&target) = type_keys.get(&(
                f.resolved_type.module_path.clone(),
                f.resolved_type.type_name.clone(),
            )) {
                edges[i].push((target, fi));
            }
        }
    }
    for (i, v) in variants.iter().enumerate() {
        let node_idx = num_records + i;
        for (ci, c) in v.constructors.iter().enumerate() {
            if let Some(&target) =
                type_keys.get(&(c.payload.module_path.clone(), c.payload.type_name.clone()))
            {
                edges[node_idx].push((target, ci));
            }
        }
    }

    // Find SCCs using iterative Tarjan's algorithm
    let sccs = find_sccs(total, &edges);

    // For each SCC with a cycle, mark the back-edges as boxed
    for scc in &sccs {
        if scc.len() < 2 && !has_self_edge(&edges, scc) {
            continue;
        }
        let scc_set: HashSet<usize> = scc.iter().copied().collect();
        for &node in scc {
            for &(target, field_idx) in &edges[node] {
                if scc_set.contains(&target) {
                    // This edge is part of a cycle — mark as boxed
                    if node < num_records {
                        records[node].fields[field_idx].resolved_type.boxed = true;
                    } else {
                        let vi = node - num_records;
                        variants[vi].constructors[field_idx].payload.boxed = true;
                    }
                }
            }
        }
    }
}

fn has_self_edge(edges: &[Vec<(usize, usize)>], scc: &[usize]) -> bool {
    if scc.len() != 1 {
        return false;
    }
    let node = scc[0];
    edges[node].iter().any(|&(target, _)| target == node)
}

fn find_sccs(n: usize, edges: &[Vec<(usize, usize)>]) -> Vec<Vec<usize>> {
    // Simple iterative Tarjan's SCC algorithm
    let mut index_counter = 0usize;
    let mut stack: Vec<usize> = Vec::new();
    let mut on_stack = vec![false; n];
    let mut index = vec![usize::MAX; n];
    let mut lowlink = vec![0usize; n];
    let mut sccs = Vec::new();

    // Simplified adjacency (just target nodes)
    let adj: Vec<Vec<usize>> = edges
        .iter()
        .map(|e| e.iter().map(|&(t, _)| t).collect())
        .collect();

    for i in 0..n {
        if index[i] == usize::MAX {
            // Iterative strongconnect
            let mut work_stack: Vec<(usize, usize)> = vec![(i, 0)];
            while let Some(&mut (v, ref mut ei)) = work_stack.last_mut() {
                if index[v] == usize::MAX {
                    index[v] = index_counter;
                    lowlink[v] = index_counter;
                    index_counter += 1;
                    stack.push(v);
                    on_stack[v] = true;
                }

                let mut recurse = false;
                while *ei < adj[v].len() {
                    let w = adj[v][*ei];
                    if index[w] == usize::MAX {
                        *ei += 1;
                        work_stack.push((w, 0));
                        recurse = true;
                        break;
                    } else if on_stack[w] {
                        lowlink[v] = lowlink[v].min(index[w]);
                    }
                    *ei += 1;
                }

                if !recurse {
                    if lowlink[v] == index[v] {
                        let mut scc = Vec::new();
                        loop {
                            let w = stack.pop().unwrap();
                            on_stack[w] = false;
                            scc.push(w);
                            if w == v {
                                break;
                            }
                        }
                        sccs.push(scc);
                    }

                    work_stack.pop();
                    if let Some(&mut (parent, _)) = work_stack.last_mut() {
                        lowlink[parent] = lowlink[parent].min(lowlink[v]);
                    }
                }
            }
        }
    }

    sccs
}

// --- SDK package filter ---

fn is_sdk_package(package: &Package) -> bool {
    if let Some(name) = package_name_from_package(package) {
        return name.starts_with("daml-prim")
            || name.starts_with("daml-stdlib")
            || name.starts_with("daml-script")
            || name.starts_with("ghc-");
    }
    false
}

// --- Main entry points ---

/// Generates Rust code from one or more DAR files.
pub fn generate_rust_code_from_dars(dar_paths: &[&str], output_path: &str) -> Result<()> {
    let dar = crate::package::parse_dars(dar_paths)?;

    // Phase 1: Collect all type representations
    let mut all_records: Vec<DamlRecordRep> = Vec::new();
    let mut all_variants: Vec<DamlVariantRep> = Vec::new();
    let mut all_enums: Vec<DamlEnumRep> = Vec::new();
    let mut all_aliases: Vec<DamlTypeAliasRep> = Vec::new();

    for (pkg_id, package) in &dar.packages {
        if is_sdk_package(package) {
            continue;
        }

        let pkg_name = package_name_from_package(package)
            .map(|n| sanitize_ident_str(&n))
            .unwrap_or_else(|| sanitize_ident_str(pkg_id));

        let ctx = TypeResolutionContext {
            current_package_name: &pkg_name,
            parsed_dar: &dar,
            interned_types: &package.interned_types,
            interned_strings: &package.interned_strings,
            interned_dotted_names: &package.interned_dotted_names,
        };

        for module in &package.modules {
            // Collect template payload type names for this module
            use crate::daml_custom_data_type_reps::record::resolve_interned_dotted_name;
            let template_type_names: HashSet<String> = module
                .templates
                .iter()
                .filter_map(|t| {
                    resolve_interned_dotted_name(t.tycon_interned_dname, package)
                        .ok()
                        .and_then(|segs| segs.last().cloned())
                })
                .collect();

            for ddt in &module.data_types {
                match &ddt.data_cons {
                    Some(DataCons::Record(_)) => {
                        let name = resolve_interned_dotted_name(ddt.name_interned_dname, package)
                            .ok()
                            .and_then(|s| s.last().cloned())
                            .unwrap_or_default();
                        let is_template = template_type_names.contains(&name);
                        if let Ok(rep) =
                            DamlRecordRep::try_from_record(ddt, module, package, &ctx, is_template)
                        {
                            all_records.push(rep);
                        }
                    }
                    Some(DataCons::Variant(_)) => {
                        if let Ok(rep) =
                            DamlVariantRep::try_from_variant(ddt, module, package, &ctx)
                        {
                            all_variants.push(rep);
                        }
                    }
                    Some(DataCons::Enum(_)) => {
                        if let Ok(rep) = DamlEnumRep::try_from_enum(ddt, module, package, &ctx) {
                            all_enums.push(rep);
                        }
                    }
                    _ => {} // Skip interfaces and None
                }
            }
            for syn in &module.synonyms {
                if let Ok(rep) = DamlTypeAliasRep::try_from_synonym(syn, module, package, &ctx) {
                    all_aliases.push(rep);
                }
            }
        }
    }

    // Phase 2: Resolve recursive types
    resolve_recursive_types(&mut all_records, &mut all_variants);

    // Phase 3: Build ModuleTree and generate tokens
    let mut tree = ModuleTree::new();
    for r in all_records {
        tree.insert_record(r);
    }
    for v in all_variants {
        tree.insert_variant(v);
    }
    for e in all_enums {
        tree.insert_enum(e);
    }
    for a in all_aliases {
        tree.insert_type_alias(a);
    }

    let tokens = tree.to_token_stream(&[]);
    let syntax_tree =
        syn::parse2(tokens).context("Failed to parse generated tokens into syntax tree")?;
    let code = prettyplease::unparse(&syntax_tree);

    let mut output = File::create(output_path)
        .with_context(|| format!("Failed to create output file '{}'", output_path))?;
    write!(output, "{}", code).with_context(|| "Failed to write generated code to output file")?;
    Ok(())
}

/// Backward-compatible wrapper for single DAR.
pub fn generate_rust_code_from_dar(dar_path: &str, output_path: &str) -> Result<()> {
    generate_rust_code_from_dars(&[dar_path], output_path)
}

/// Legacy entry point (kept for existing tests).
pub fn generate_rust_structs_from_dar(dar_path: &str, output_path: &str) -> Result<()> {
    generate_rust_code_from_dar(dar_path, output_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::info;

    #[test]
    fn test_generate_rust_structs_from_dar() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        info!("Crate root: {}", crate_root);
        let dar_path = std::path::PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-ticketoffer")
            .join("main")
            .join(".daml")
            .join("dist")
            .join("daml-ticketoffer-0.0.1.dar")
            .canonicalize()
            .expect("Failed to canonicalize package_root");
        let output_path = std::path::PathBuf::from(&crate_root)
            .join("generated")
            .join("ticketoffer_structs.rs");
        let result = generate_rust_structs_from_dar(
            dar_path.to_str().expect("DAR path is not valid UTF-8"),
            output_path
                .to_str()
                .expect("Output path is not valid UTF-8"),
        );
        assert!(
            result.is_ok(),
            "Failed to generate Rust structs from DAR: {:?}",
            result.err()
        );
        let contents = std::fs::read_to_string(output_path).expect("Output file not found");
        assert!(!contents.is_empty(), "Output file is empty");
    }

    #[test]
    fn test_generate_from_nested_dar() {
        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let dar_path = std::path::PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-nested-test")
            .join("main")
            .join(".daml")
            .join("dist")
            .join("daml-nested-test-0.0.1.dar")
            .canonicalize()
            .expect("Failed to canonicalize nested test DAR path");
        let output_path = std::path::PathBuf::from(&crate_root)
            .join("generated")
            .join("nested_test_structs.rs");
        let result = generate_rust_code_from_dar(
            dar_path.to_str().expect("DAR path is not valid UTF-8"),
            output_path
                .to_str()
                .expect("Output path is not valid UTF-8"),
        );
        assert!(
            result.is_ok(),
            "Failed to generate Rust code from nested DAR: {:?}",
            result.err()
        );
        let contents = std::fs::read_to_string(&output_path).expect("Output file not found");
        assert!(!contents.is_empty(), "Output file is empty");
        println!("Generated nested test output:\n{}", contents);
    }
}
