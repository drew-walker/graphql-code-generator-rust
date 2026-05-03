use serde_json::Value;

use crate::transitional_plugins::introspection::{
    is_non_null, named_type, pascal_case, root_type_name, sorted_fields, type_name,
};

pub fn output(introspection: &Value, objects: &[Value]) -> String {
    let mut out = String::new();
    out.push_str(FLOW_RESOLVER_HELPERS);
    out.push_str(&resolver_maps(introspection, objects));
    out
}

const FLOW_RESOLVER_HELPERS: &str = r#"

export type Resolver<Result, Parent = {}, Context = {}, Args = {}> = (
  parent: Parent,
  args: Args,
  context: Context,
  info: GraphQLResolveInfo,
) => Promise<Result> | Result;

export type SubscriptionSubscribeFn<Result, Parent, Context, Args> = (
  parent: Parent,
  args: Args,
  context: Context,
  info: GraphQLResolveInfo,
) => AsyncIterator<Result> | Promise<AsyncIterator<Result>>;

export type SubscriptionResolveFn<Result, Parent, Context, Args> = (
  parent: Parent,
  args: Args,
  context: Context,
  info: GraphQLResolveInfo,
) => Result | Promise<Result>;

export interface SubscriptionSubscriberObject<Result, Key: string, Parent, Context, Args> {
  subscribe: SubscriptionSubscribeFn<{ [key: Key]: Result }, Parent, Context, Args>;
  resolve?: SubscriptionResolveFn<Result, { [key: Key]: Result }, Context, Args>;
}

export interface SubscriptionResolverObject<Result, Parent, Context, Args> {
  subscribe: SubscriptionSubscribeFn<mixed, Parent, Context, Args>;
  resolve: SubscriptionResolveFn<Result, mixed, Context, Args>;
}

export type SubscriptionObject<Result, Key: string, Parent, Context, Args> =
  | SubscriptionSubscriberObject<Result, Key, Parent, Context, Args>
  | SubscriptionResolverObject<Result, Parent, Context, Args>;

export type SubscriptionResolver<Result, Key: string, Parent = {}, Context = {}, Args = {}> =
  | ((...args: Array<any>) => SubscriptionObject<Result, Key, Parent, Context, Args>)
  | SubscriptionObject<Result, Key, Parent, Context, Args>;

export type TypeResolveFn<Types, Parent = {}, Context = {}> = (
  parent: Parent,
  context: Context,
  info: GraphQLResolveInfo,
) => ?Types | Promise<?Types>;

export type IsTypeOfResolverFn<T = {}, Context = {}> = (
  obj: T,
  context: Context,
  info: GraphQLResolveInfo,
) => boolean | Promise<boolean>;

export type NextResolverFn<T> = () => Promise<T>;

export type DirectiveResolverFn<Result = {}, Parent = {}, Args = {}, Context = {}> = (
  next: NextResolverFn<Result>,
  parent: Parent,
  args: Args,
  context: Context,
  info: GraphQLResolveInfo,
) => Result | Promise<Result>;

export type ResolverTypeWrapper<T> = Promise<T> | T;
"#;

fn resolver_maps(introspection: &Value, objects: &[Value]) -> String {
    let root_query =
        root_type_name(introspection, "queryType").unwrap_or_else(|| "Query".to_string());
    let mut out = String::new();
    out.push_str("\n/** Mapping between all available schema types and the resolvers types */\n");
    out.push_str("export type ResolversTypes = {\n");
    let mut resolver_type_entries = vec![
        "Boolean: ResolverTypeWrapper<$ElementType<Scalars, 'Boolean'>>".to_string(),
        "Int: ResolverTypeWrapper<$ElementType<Scalars, 'Int'>>".to_string(),
        "String: ResolverTypeWrapper<$ElementType<Scalars, 'String'>>".to_string(),
    ];
    for object in objects {
        let name = type_name(object);
        let mapped = if name == root_query { "{}" } else { name };
        resolver_type_entries.push(format!("{name}: ResolverTypeWrapper<{mapped}>"));
    }
    resolver_type_entries.sort();
    for entry in resolver_type_entries {
        out.push_str(&format!("  {entry},\n"));
    }
    out.push_str("};\n");

    out.push_str("\n/** Mapping between all available schema types and the resolvers parents */\n");
    out.push_str("export type ResolversParentTypes = {\n");
    let mut parent_type_entries = vec![
        "Boolean: $ElementType<Scalars, 'Boolean'>".to_string(),
        "Int: $ElementType<Scalars, 'Int'>".to_string(),
        "String: $ElementType<Scalars, 'String'>".to_string(),
    ];
    for object in objects {
        let name = type_name(object);
        let mapped = if name == root_query { "{}" } else { name };
        parent_type_entries.push(format!("{name}: {mapped}"));
    }
    parent_type_entries.sort();
    for entry in parent_type_entries {
        out.push_str(&format!("  {entry},\n"));
    }
    out.push_str("};\n");

    for object in objects {
        let name = type_name(object);
        out.push('\n');
        out.push_str(&format!(
            "export type {name}Resolvers<\n  ContextType = any,\n  ParentType = $ElementType<ResolversParentTypes, '{name}'>,\n> = {{\n"
        ));
        for field in sorted_fields(object) {
            let field_name = field.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = field
                .get("args")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let result_ty = flow_resolver_type_ref(field.get("type").unwrap_or(&Value::Null));
            if args.is_empty() {
                out.push_str(&format!(
                    "  {field_name}?: Resolver<{result_ty}, ParentType, ContextType>,\n"
                ));
            } else {
                let required = args
                    .iter()
                    .filter_map(|arg| {
                        let ty = arg.get("type")?;
                        if is_non_null(ty) {
                            arg.get("name")?.as_str()
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                let args_ty = if required.is_empty() {
                    format!("{name}{}Args", pascal_case(field_name))
                } else {
                    let fields = required
                        .iter()
                        .map(|name| format!("{name}: *"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "$RequireFields<{name}{}Args, {{ {fields} }}>",
                        pascal_case(field_name)
                    )
                };
                out.push_str(&format!(
                    "  {field_name}?: Resolver<\n    {result_ty},\n    ParentType,\n    ContextType,\n    {args_ty},\n  >,\n"
                ));
            }
        }
        if name != root_query {
            out.push_str("  __isTypeOf?: IsTypeOfResolverFn<ParentType, ContextType>,\n");
        }
        out.push_str("};\n");
    }

    out.push_str("\nexport type Resolvers<ContextType = any> = {\n");
    for object in objects {
        let name = type_name(object);
        out.push_str(&format!("  {name}?: {name}Resolvers<ContextType>,\n"));
    }
    out.push_str("};\n");
    out
}

fn flow_resolver_type_ref(type_ref: &Value) -> String {
    flow_resolver_type_ref_inner(type_ref, true)
}

fn flow_resolver_type_ref_inner(type_ref: &Value, nullable: bool) -> String {
    match type_ref.get("kind").and_then(|v| v.as_str()).unwrap_or("") {
        "NON_NULL" => {
            flow_resolver_type_ref_inner(type_ref.get("ofType").unwrap_or(&Value::Null), false)
        }
        "LIST" => {
            let inner = flow_resolver_type_ref(type_ref.get("ofType").unwrap_or(&Value::Null));
            let list = format!("Array<{inner}>");
            if nullable { format!("?{list}") } else { list }
        }
        _ => {
            let named = format!("$ElementType<ResolversTypes, '{}'>", named_type(type_ref));
            if nullable { format!("?{named}") } else { named }
        }
    }
}
