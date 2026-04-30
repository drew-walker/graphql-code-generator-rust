//! Port of `packages/plugins/typescript/operations/src/visitor.ts` (minimal subset).

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context as _, Result};
use graphql_parser::query::{
    Definition, Document, FragmentDefinition, OperationDefinition, Selection, SelectionSet,
    Type as AstType, TypeCondition,
};
use plugin_helpers::schema_input::SchemaGenerationInput;
use plugin_helpers::types::{ComplexPluginOutput, DocumentFile};
use serde_json::Value;

use crate::config::TypeScriptDocumentsPluginConfig;
use crate::ts_operation_variables_to_object::TypeScriptOperationVariablesToObject;
use crate::ts_selection_set_processor::format_selections;

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

fn scalar_output_ts(name: &str) -> String {
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
) -> Result<String> {
    match type_ref {
        TypeRef::NonNull(inner) => output_ts(inner, base_ts_for_named),
        TypeRef::List(inner) => Ok(format!("Array<{}>", output_ts(inner, base_ts_for_named)?)),
        TypeRef::Named(name) => Ok(format!(
            "{} | null",
            output_ts_nonnull(name, base_ts_for_named)?
        )),
    }
}

fn output_field(
    optionality_ref: &TypeRef,
    base_ts_for_named: &impl Fn(&str) -> Result<String>,
) -> Result<(bool, String)> {
    let optional = !matches!(optionality_ref, TypeRef::NonNull(_));
    let ts = match optionality_ref {
        TypeRef::NonNull(inner) => match inner.as_ref() {
            TypeRef::List(l) => Ok(format!("Array<{}>", output_ts(l, base_ts_for_named)?)),
            TypeRef::Named(name) => base_ts_for_named(name),
            TypeRef::NonNull(_) => output_ts(inner, base_ts_for_named),
        },
        TypeRef::List(inner) => Ok(format!(
            "Array<{}> | null",
            output_ts(inner, base_ts_for_named)?
        )),
        TypeRef::Named(name) => Ok(format!("{} | null", base_ts_for_named(name)?)),
    }?;
    Ok((optional, ts))
}

pub struct TypeScriptDocumentsVisitor<'a> {
    config: &'a TypeScriptDocumentsPluginConfig,
    documents: &'a [DocumentFile],
    types_by_name: HashMap<String, Value>,
}

struct CollectSelectionsCtx<'a> {
    primitive: &'a mut Vec<String>,
    links: &'a mut HashMap<String, (TypeRef, SelectionSet<'static, String>)>,
    link_order: &'a mut Vec<String>,
    seen_primitive: &'a mut HashSet<String>,
    seen_link: &'a mut HashSet<String>,
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
                    let ts =
                        self.selection_set_object_ts(&type_name, &frag.selection_set, &fragments)?;
                    content_parts.push(format!("export type {name}Fragment = {ts};"));
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
        let transformer = TypeScriptOperationVariablesToObject::new(&is_enum);
        Ok(transformer.transform_operation_variables(op))
    }

    fn selection_set_object_ts(
        &self,
        parent_type: &str,
        selection_set: &SelectionSet<'static, String>,
        fragments: &BTreeMap<String, FragmentDefinition<'static, String>>,
    ) -> Result<String> {
        let mut primitive: Vec<String> = Vec::new();
        // Link fields need selection-set merging (e.g. `repository { ... }` from multiple fragments).
        // We store them as merged AST selection sets and render once at the end.
        let mut link_order: Vec<String> = Vec::new();
        let mut links: HashMap<String, (TypeRef, SelectionSet<'static, String>)> = HashMap::new();

        // Keep primitive field insertion-order while deduping.
        let mut seen_primitive: HashSet<String> = HashSet::new();
        let mut seen_link: HashSet<String> = HashSet::new();

        // `__typename` first.
        primitive.push(format!("__typename?: '{parent_type}'"));
        seen_primitive.insert("__typename".to_string());

        self.collect_selections_into(
            parent_type,
            &selection_set.items,
            fragments,
            &mut CollectSelectionsCtx {
                primitive: &mut primitive,
                links: &mut links,
                link_order: &mut link_order,
                seen_primitive: &mut seen_primitive,
                seen_link: &mut seen_link,
            },
        )?;

        // Render links (merged) after primitives (matches upstream pipeline ordering).
        let mut selections = primitive;
        for name in link_order {
            if let Some((type_ref, merged_ss)) = links.remove(&name) {
                let base_ts_for_named = |tn: &str| -> Result<String> {
                    if self.is_scalar(tn) {
                        return Ok(scalar_output_ts(tn));
                    }
                    if self.is_enum(tn) {
                        return Ok(tn.to_string());
                    }
                    self.selection_set_object_ts(tn, &merged_ss, fragments)
                };
                let (optional, ts) = output_field(&type_ref, &base_ts_for_named)?;
                let q = if optional { "?" } else { "" };
                selections.push(format!("{name}{q}: {ts}"));
            }
        }

        if self.config.print_fields_on_new_lines {
            return Ok(format_selections(&selections));
        }
        Ok(format!("{{ {} }}", selections.join(", ")))
    }

    fn collect_selections_into(
        &self,
        parent_type: &str,
        items: &[Selection<'static, String>],
        fragments: &BTreeMap<String, FragmentDefinition<'static, String>>,
        ctx: &mut CollectSelectionsCtx<'_>,
    ) -> Result<()> {
        // Inline fragments are applied first (matches fixtures for `... on Repository { ... }`).
        for sel in items {
            if let Selection::InlineFragment(inline) = sel {
                let type_name = inline
                    .type_condition
                    .as_ref()
                    .map(|tc| match tc {
                        TypeCondition::On(t) => t.as_str(),
                    })
                    .unwrap_or(parent_type);
                self.collect_selections_into(
                    type_name,
                    &inline.selection_set.items,
                    fragments,
                    ctx,
                )?;
            }
        }

        for sel in items {
            match sel {
                Selection::Field(f) => {
                    let field_name = f.name.clone();
                    let out_name = f.alias.clone().unwrap_or_else(|| field_name.clone());
                    let (field_type_ref, named) = self.field_type(parent_type, &field_name)?;

                    if f.selection_set.items.is_empty() {
                        let base_ts_for_named = |tn: &str| -> Result<String> {
                            if self.is_scalar(tn) {
                                return Ok(scalar_output_ts(tn));
                            }
                            if self.is_enum(tn) {
                                return Ok(tn.to_string());
                            }
                            Ok("any".to_string())
                        };
                        let (optional, ts) = output_field(&field_type_ref, &base_ts_for_named)?;
                        if ctx.seen_primitive.insert(out_name.clone()) {
                            let q = if optional { "?" } else { "" };
                            ctx.primitive.push(format!("{out_name}{q}: {ts}"));
                        }
                        continue;
                    }

                    // Link field: merge selection sets by field name.
                    if ctx.seen_link.insert(out_name.clone()) {
                        ctx.link_order.push(out_name.clone());
                        ctx.links.insert(
                            out_name.clone(),
                            (
                                field_type_ref.clone(),
                                SelectionSet {
                                    span: f.selection_set.span,
                                    items: f.selection_set.items.clone(),
                                },
                            ),
                        );
                    } else if let Some((_tr0, ss)) = ctx.links.get_mut(&out_name) {
                        ss.items.extend(f.selection_set.items.clone());
                    }

                    let _ = named;
                }
                Selection::FragmentSpread(spread) => {
                    if let Some(frag) = fragments.get(&spread.fragment_name) {
                        let type_name = match &frag.type_condition {
                            TypeCondition::On(t) => t.as_str(),
                        };
                        self.collect_selections_into(
                            type_name,
                            &frag.selection_set.items,
                            fragments,
                            ctx,
                        )?;
                    }
                }
                Selection::InlineFragment(_) => {}
            }
        }
        Ok(())
    }

    fn field_type(&self, parent_type: &str, field: &str) -> Result<(TypeRef, String)> {
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

    fn named_type(&self, tr: &TypeRef) -> String {
        match tr {
            TypeRef::Named(n) => n.clone(),
            TypeRef::List(inner) | TypeRef::NonNull(inner) => self.named_type(inner),
        }
    }

    fn is_scalar(&self, name: &str) -> bool {
        matches!(name, "ID" | "String" | "Boolean" | "Int" | "Float")
            || self
                .types_by_name
                .get(name)
                .and_then(|t| t.get("kind").and_then(|k| k.as_str()))
                == Some("SCALAR")
    }

    fn is_enum(&self, name: &str) -> bool {
        self.types_by_name
            .get(name)
            .and_then(|t| t.get("kind").and_then(|k| k.as_str()))
            == Some("ENUM")
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
