//! Structural extraction of selection-set-to-object logic.
//!
//! This is a step toward upstream's `SelectionSetToObject` + processor pipeline shape, while
//! preserving current behavior.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use graphql_parser::query::{FragmentDefinition, Selection, SelectionSet, TypeCondition};

use crate::visitor::{
    CollectSelectionsCtx, TypeRef, TypeScriptDocumentsVisitor, output_field, scalar_output_ts,
};

pub(crate) fn selection_set_object_ts(
    v: &TypeScriptDocumentsVisitor<'_>,
    parent_type: &str,
    selection_set: &SelectionSet<'static, String>,
    fragments: &BTreeMap<String, FragmentDefinition<'static, String>>,
) -> Result<String> {
    if v.is_abstract_type(parent_type) {
        let possible = v.possible_types(parent_type);
        if possible.is_empty() {
            return Ok("{ __typename?: 'never' }".to_string());
        }
        let mut variants: Vec<String> = Vec::new();
        for t in possible {
            variants.push(selection_set_object_ts(v, &t, selection_set, fragments)?);
        }
        return Ok(variants.join(" | "));
    }

    let mut primitive: Vec<String> = Vec::new();
    let mut link_order: Vec<String> = Vec::new();
    let mut links: HashMap<String, (TypeRef, SelectionSet<'static, String>)> = HashMap::new();

    let mut seen_primitive: HashSet<String> = HashSet::new();
    let mut seen_link: HashSet<String> = HashSet::new();

    if v.config.immutable_types {
        primitive.push(format!("readonly __typename?: '{parent_type}'"));
    } else {
        primitive.push(format!("__typename?: '{parent_type}'"));
    }
    seen_primitive.insert("__typename".to_string());

    collect_selections_into(
        v,
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

    let mut selections = primitive;
    for name in link_order {
        if let Some((type_ref, merged_ss)) = links.remove(&name) {
            let base_ts_for_named = |tn: &str| -> Result<String> {
                if v.is_scalar(tn) {
                    return Ok(scalar_output_ts(tn));
                }
                if v.is_enum(tn) {
                    return Ok(tn.to_string());
                }
                selection_set_object_ts(v, tn, &merged_ss, fragments)
            };
            let (optional, ts) =
                output_field(&type_ref, &base_ts_for_named, v.config.immutable_types)?;
            let q = if optional && !v.config.avoid_optionals {
                "?"
            } else {
                ""
            };
            let ro = if v.config.immutable_types {
                "readonly "
            } else {
                ""
            };
            selections.push(format!("{ro}{name}{q}: {ts}"));
        }
    }

    if v.config.print_fields_on_new_lines {
        return Ok(crate::ts_selection_set_processor::format_selections(
            &selections,
        ));
    }
    Ok(format!("{{ {} }}", selections.join(", ")))
}

pub(crate) fn collect_selections_into(
    v: &TypeScriptDocumentsVisitor<'_>,
    parent_type: &str,
    items: &[Selection<'static, String>],
    fragments: &BTreeMap<String, FragmentDefinition<'static, String>>,
    ctx: &mut CollectSelectionsCtx<'_>,
) -> Result<()> {
    for sel in items {
        if let Selection::InlineFragment(inline) = sel {
            let type_name = inline
                .type_condition
                .as_ref()
                .map(|tc| match tc {
                    TypeCondition::On(t) => t.as_str(),
                })
                .unwrap_or(parent_type);
            if !v.does_type_condition_apply(parent_type, inline.type_condition.as_ref()) {
                continue;
            }
            collect_selections_into(v, type_name, &inline.selection_set.items, fragments, ctx)?;
        }
    }

    for sel in items {
        match sel {
            Selection::Field(f) => {
                let field_name = f.name.clone();
                let out_name = f.alias.clone().unwrap_or_else(|| field_name.clone());
                let (field_type_ref, _named) = v.field_type(parent_type, &field_name)?;

                if f.selection_set.items.is_empty() {
                    let base_ts_for_named = |tn: &str| -> Result<String> {
                        if v.is_scalar(tn) {
                            return Ok(scalar_output_ts(tn));
                        }
                        if v.is_enum(tn) {
                            return Ok(tn.to_string());
                        }
                        Ok("any".to_string())
                    };
                    let (mut optional, ts) = output_field(
                        &field_type_ref,
                        &base_ts_for_named,
                        v.config.immutable_types,
                    )?;
                    if has_conditional_directives(&f.directives) {
                        optional = true;
                    }
                    if ctx.seen_primitive.insert(out_name.clone()) {
                        let q = if optional && !v.config.avoid_optionals {
                            "?"
                        } else {
                            ""
                        };
                        let ro = if v.config.immutable_types {
                            "readonly "
                        } else {
                            ""
                        };
                        ctx.primitive.push(format!("{ro}{out_name}{q}: {ts}"));
                    }
                    continue;
                }

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
            }
            Selection::FragmentSpread(spread) => {
                if let Some(frag) = fragments.get(&spread.fragment_name)
                    && v.does_type_condition_apply(parent_type, Some(&frag.type_condition))
                {
                    collect_selections_into(
                        v,
                        parent_type,
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

fn has_conditional_directives(
    directives: &[graphql_parser::query::Directive<'static, String>],
) -> bool {
    directives
        .iter()
        .any(|d| d.name == "include" || d.name == "skip")
}
