//! Port of `packages/plugins/typescript/operations/src/visitor.ts` (minimal subset).

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context as _, Result};
use graphql_parser::query::{
    Definition, Document, FragmentDefinition, OperationDefinition, SelectionSet, Type as AstType,
    TypeCondition,
};
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::{ComplexPluginOutput, DocumentFile};
use serde_json::Value;

use crate::config::TypeScriptDocumentsPluginConfig;
use crate::ts_operation_variables_to_object::TypeScriptOperationVariablesToObject;
use crate::ts_selection_set_to_object as selection_set_to_object;

#[derive(Debug, Clone)]
pub(crate) enum TypeRef {
    Named(String),
    List(Box<TypeRef>),
    NonNull(Box<TypeRef>),
}

fn parse_type_ref(v: &Value) -> Result<TypeRef> {
    let kind = v
        .get("kind")
        .and_then(|k| k.as_str())
        .context("type ref missing kind")?;
    match kind {
        "NON_NULL" => Ok(TypeRef::NonNull(Box::new(parse_type_ref(
            v.get("ofType").context("NON_NULL missing ofType")?,
        )?))),
        "LIST" => Ok(TypeRef::List(Box::new(parse_type_ref(
            v.get("ofType").context("LIST missing ofType")?,
        )?))),
        _ => {
            let name = v
                .get("name")
                .and_then(|n| n.as_str())
                .context("named type ref missing name")?;
            Ok(TypeRef::Named(name.to_string()))
        }
    }
}

pub(crate) fn parse_ast_type_ref(t: &AstType<String>) -> TypeRef {
    match t {
        AstType::NamedType(n) => TypeRef::Named(n.clone()),
        AstType::ListType(inner) => TypeRef::List(Box::new(parse_ast_type_ref(inner))),
        AstType::NonNullType(inner) => TypeRef::NonNull(Box::new(parse_ast_type_ref(inner))),
    }
}

fn to_pascal_case(name: &str) -> String {
    let mut out = String::new();
    let mut upper_next = true;
    for ch in name.chars() {
        if ch == '_' || ch == '-' || ch == ' ' {
            upper_next = true;
            continue;
        }
        if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

pub(crate) fn scalar_output_ts(name: &str) -> String {
    match name {
        "ID" | "String" => "string".to_string(),
        "Boolean" => "boolean".to_string(),
        "Int" | "Float" => "number".to_string(),
        other => format!("Scalars['{other}']['output']"),
    }
}

fn output_ts_nonnull(
    base_named: &str,
    base_ts_for_named: &impl Fn(&str) -> Result<String>,
) -> Result<String> {
    base_ts_for_named(base_named)
}

fn output_ts(
    type_ref: &TypeRef,
    base_ts_for_named: &impl Fn(&str) -> Result<String>,
    immutable_types: bool,
) -> Result<String> {
    let array_ty = if immutable_types {
        "ReadonlyArray"
    } else {
        "Array"
    };
    match type_ref {
        TypeRef::NonNull(inner) => output_ts(inner, base_ts_for_named, immutable_types),
        TypeRef::List(inner) => Ok(format!(
            "{array_ty}<{}>",
            output_ts(inner, base_ts_for_named, immutable_types)?
        )),
        TypeRef::Named(name) => Ok(format!(
            "{} | null",
            output_ts_nonnull(name, base_ts_for_named)?
        )),
    }
}

pub(crate) fn output_field(
    optionality_ref: &TypeRef,
    base_ts_for_named: &impl Fn(&str) -> Result<String>,
    immutable_types: bool,
) -> Result<(bool, String)> {
    let array_ty = if immutable_types {
        "ReadonlyArray"
    } else {
        "Array"
    };
    let optional = !matches!(optionality_ref, TypeRef::NonNull(_));
    let ts = match optionality_ref {
        TypeRef::NonNull(inner) => match inner.as_ref() {
            TypeRef::List(l) => Ok(format!(
                "{array_ty}<{}>",
                output_ts(l, base_ts_for_named, immutable_types)?
            )),
            TypeRef::Named(name) => base_ts_for_named(name),
            TypeRef::NonNull(_) => output_ts(inner, base_ts_for_named, immutable_types),
        },
        TypeRef::List(inner) => Ok(format!(
            "{array_ty}<{}> | null",
            output_ts(inner, base_ts_for_named, immutable_types)?
        )),
        TypeRef::Named(name) => Ok(format!("{} | null", base_ts_for_named(name)?)),
    }?;
    Ok((optional, ts))
}

pub struct TypeScriptDocumentsVisitor<'a> {
    pub(crate) config: &'a TypeScriptDocumentsPluginConfig,
    documents: &'a [DocumentFile],
    types_by_name: HashMap<String, Value>,
}

pub(crate) struct CollectSelectionsCtx<'a> {
    pub(crate) primitive: &'a mut Vec<String>,
    pub(crate) links: &'a mut HashMap<String, (TypeRef, SelectionSet<'static, String>)>,
    pub(crate) link_order: &'a mut Vec<String>,
    pub(crate) seen_primitive: &'a mut HashSet<String>,
    pub(crate) seen_link: &'a mut HashSet<String>,
}

impl<'a> TypeScriptDocumentsVisitor<'a> {
    pub fn new(
        schema: &'a SchemaGenerationInput,
        config: &'a TypeScriptDocumentsPluginConfig,
        documents: &'a [DocumentFile],
    ) -> Self {
        let mut types_by_name = HashMap::new();
        if let Some(arr) = schema.introspection.get("types").and_then(|t| t.as_array()) {
            for t in arr {
                if let Some(name) = t.get("name").and_then(|n| n.as_str()) {
                    types_by_name.insert(name.to_string(), t.clone());
                }
            }
        }
        Self {
            config,
            documents,
            types_by_name,
        }
    }

    pub fn generate(&self) -> Result<ComplexPluginOutput> {
        let merged = self.merge_documents();
        let fragments = collect_fragments(&merged);

        let mut content_parts: Vec<String> = Vec::new();

        // Emit in definition order (closer to upstream visit-driven behavior).
        for def in &merged.definitions {
            match def {
                Definition::Fragment(frag) => {
                    let name = frag.name.clone();
                    let type_name = match &frag.type_condition {
                        TypeCondition::On(t) => t.clone(),
                    };
                    if self.is_abstract_type(&type_name) {
                        let possible = self.possible_types(&type_name);
                        let mut members: Vec<String> = Vec::new();
                        for t in possible {
                            let inner =
                                self.selection_set_object_ts(&t, &frag.selection_set, &fragments)?;
                            let inner_name = format!("{name}_{t}_Fragment");
                            content_parts.push(format!("type {inner_name} = {inner};"));
                            members.push(inner_name);
                        }
                        content_parts.push(format!(
                            "export type {name}Fragment = {};",
                            members.join(" | ")
                        ));
                    } else {
                        let ts = self.selection_set_object_ts(
                            &type_name,
                            &frag.selection_set,
                            &fragments,
                        )?;
                        content_parts.push(format!("export type {name}Fragment = {ts};"));
                    }
                }
                Definition::Operation(op) => {
                    let (op_name, op_kind, selection_set) = match op {
                        OperationDefinition::Query(q) => {
                            (q.name.clone(), "Query", &q.selection_set)
                        }
                        OperationDefinition::Mutation(m) => {
                            (m.name.clone(), "Mutation", &m.selection_set)
                        }
                        OperationDefinition::Subscription(s) => {
                            (s.name.clone(), "Subscription", &s.selection_set)
                        }
                        OperationDefinition::SelectionSet(_) => continue,
                    };

                    let Some(name) = op_name else { continue };

                    let op_base = format!("{}{}", to_pascal_case(&name), op_kind);
                    let vars_name = format!("{op_base}Variables");
                    let result_name = op_base;

                    let variables_ts = self.operation_variables_ts(op)?;
                    content_parts.push(format!("export type {vars_name} = {variables_ts};"));

                    let result_ts =
                        self.selection_set_object_ts(op_kind, selection_set, &fragments)?;
                    content_parts.push(format!("export type {result_name} = {result_ts};"));
                }
            }
        }

        let mut content = content_parts.join("\n\n");
        if self.config.global_namespace && !content.is_empty() {
            content = format!("\n    declare global {{\n      {content}\n    }}");
        }

        Ok(ComplexPluginOutput {
            prepend: self.get_imports(),
            content,
            append: vec![],
        })
    }

    pub fn get_imports(&self) -> Vec<String> {
        // Upstream `getImports()` depends on `inlineFragmentTypes` / fragmentImports.
        // We expose the method for parity; return empty until those config surfaces are ported.
        vec![]
    }

    pub fn get_global_declarations(&self, _no_export: bool) -> Vec<String> {
        // Upstream `getGlobalDeclarations` controls `declare global` typing helpers.
        // Return empty until the related config surface is ported.
        vec![]
    }

    fn merge_documents(&self) -> Document<'static, String> {
        let mut defs: Vec<Definition<'static, String>> = Vec::new();
        for d in self.documents {
            for def in &d.document.definitions {
                defs.push(def.clone());
            }
        }
        Document { definitions: defs }
    }

    fn operation_variables_ts(&self, op: &OperationDefinition<'static, String>) -> Result<String> {
        let is_enum = |n: &str| self.is_enum(n);
        let is_scalar = |n: &str| self.is_scalar(n);
        let transformer = TypeScriptOperationVariablesToObject::new(
            &is_enum,
            &is_scalar,
            self.config.immutable_types,
        );
        Ok(transformer.transform_operation_variables(op, self.config.avoid_optionals))
    }

    fn selection_set_object_ts(
        &self,
        parent_type: &str,
        selection_set: &SelectionSet<'static, String>,
        fragments: &BTreeMap<String, FragmentDefinition<'static, String>>,
    ) -> Result<String> {
        selection_set_to_object::selection_set_object_ts(
            self,
            parent_type,
            selection_set,
            fragments,
        )
    }

    pub(crate) fn field_type(&self, parent_type: &str, field: &str) -> Result<(TypeRef, String)> {
        let t = self
            .types_by_name
            .get(parent_type)
            .with_context(|| format!("unknown parent type `{parent_type}`"))?;
        if let Some(fields) = t.get("fields").and_then(|f| f.as_array()) {
            for f in fields {
                if f.get("name").and_then(|n| n.as_str()) == Some(field) {
                    let tr = parse_type_ref(f.get("type").context("field missing type")?)?;
                    let named = self.named_type(&tr);
                    return Ok((tr, named));
                }
            }
        }
        anyhow::bail!("unknown field `{parent_type}.{field}`")
    }

    pub(crate) fn named_type(&self, tr: &TypeRef) -> String {
        match tr {
            TypeRef::Named(n) => n.clone(),
            TypeRef::List(inner) | TypeRef::NonNull(inner) => self.named_type(inner),
        }
    }

    pub(crate) fn is_scalar(&self, name: &str) -> bool {
        matches!(name, "ID" | "String" | "Boolean" | "Int" | "Float")
            || self
                .types_by_name
                .get(name)
                .and_then(|t| t.get("kind").and_then(|k| k.as_str()))
                == Some("SCALAR")
    }

    pub(crate) fn is_enum(&self, name: &str) -> bool {
        self.types_by_name
            .get(name)
            .and_then(|t| t.get("kind").and_then(|k| k.as_str()))
            == Some("ENUM")
    }

    pub(crate) fn is_abstract_type(&self, name: &str) -> bool {
        matches!(
            self.types_by_name
                .get(name)
                .and_then(|t| t.get("kind").and_then(|k| k.as_str())),
            Some("INTERFACE") | Some("UNION")
        )
    }

    pub(crate) fn possible_types(&self, name: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .types_by_name
            .get(name)
            .and_then(|t| t.get("possibleTypes"))
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    pub(crate) fn does_type_condition_apply(
        &self,
        concrete_type: &str,
        condition: Option<&TypeCondition<'static, String>>,
    ) -> bool {
        let Some(condition) = condition else {
            return true;
        };
        let TypeCondition::On(t) = condition;
        if t == concrete_type {
            return true;
        }
        if self.is_abstract_type(t) {
            return self.possible_types(t).iter().any(|pt| pt == concrete_type);
        }
        false
    }
}

fn collect_fragments(
    doc: &Document<'static, String>,
) -> BTreeMap<String, FragmentDefinition<'static, String>> {
    let mut out = BTreeMap::new();
    for def in &doc.definitions {
        if let Definition::Fragment(f) = def {
            out.insert(f.name.clone(), f.clone());
        }
    }
    out
}
