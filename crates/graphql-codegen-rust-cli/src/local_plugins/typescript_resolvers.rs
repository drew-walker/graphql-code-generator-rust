use plugin_helpers::types::ComplexPluginOutput;
use serde_json::Value;

mod config;
mod visitor;

use super::introspection::{
    is_non_null, pascal_case, root_type_name, schema_object_types, sorted_fields, type_name,
};
use config::{ParsedResolversConfig, is_custom_scalar, split_external_mapper};
use visitor::{resolver_scalar_names, ts_resolver_type_ref};

pub fn plugin(
    introspection: &Value,
    config: &serde_json::Map<String, Value>,
) -> ComplexPluginOutput {
    let objects = schema_object_types(introspection);
    let root_query = root_type_name(introspection, "queryType");
    let root_mutation = root_type_name(introspection, "mutationType");
    let root_subscription = root_type_name(introspection, "subscriptionType");
    let parsed_config = ParsedResolversConfig::from_map(config);
    let scalar_names = resolver_scalar_names(introspection);
    let mut out = String::new();
    if parsed_config.use_index_signature {
        out.push_str(
            "export type WithIndex<TObject> = TObject & Record<string, any>;\nexport type ResolversObject<TObject> = WithIndex<TObject>;\n\n",
        );
    }
    out.push_str("export type ResolverTypeWrapper<T> = Promise<T> | T;\n\n");
    out.push_str(TS_RESOLVER_HELPERS);
    if !parsed_config.no_schema_stitching {
        out = out.replace(
            "export type Resolver<",
            "\nexport type LegacyStitchingResolver<TResult, TParent, TContext, TArgs> = {\n  fragment: string;\n  resolve: ResolverFn<TResult, TParent, TContext, TArgs>;\n};\n\nexport type NewStitchingResolver<TResult, TParent, TContext, TArgs> = {\n  selectionSet: string | ((fieldNode: FieldNode) => SelectionSetNode);\n  resolve: ResolverFn<TResult, TParent, TContext, TArgs>;\n};\nexport type StitchingResolver<TResult, TParent, TContext, TArgs> =\n  | LegacyStitchingResolver<TResult, TParent, TContext, TArgs>\n  | NewStitchingResolver<TResult, TParent, TContext, TArgs>;\nexport type Resolver<",
        );
        out = out.replace(
            "  | ResolverWithResolve<TResult, TParent, TContext, TArgs>;",
            "  | ResolverWithResolve<TResult, TParent, TContext, TArgs>\n  | StitchingResolver<TResult, TParent, TContext, TArgs>;",
        );
    }

    out.push_str("\n/** Mapping between all available schema types and the resolvers types */\n");
    out.push_str(if parsed_config.use_index_signature {
        "export type ResolversTypes = ResolversObject<{\n"
    } else {
        "export type ResolversTypes = {\n"
    });
    let mut resolver_type_entries = scalar_names
        .iter()
        .map(|scalar| format!("{scalar}: ResolverTypeWrapper<Scalars['{scalar}']['output']>"))
        .collect::<Vec<_>>();
    for object in &objects {
        let name = type_name(object);
        let mapped = if Some(name.to_string()) == root_query
            || Some(name.to_string()) == root_mutation
            || Some(name.to_string()) == root_subscription
        {
            "Record<PropertyKey, never>".to_string()
        } else {
            name.to_string()
        };
        resolver_type_entries.push(format!("{name}: ResolverTypeWrapper<{mapped}>"));
    }
    resolver_type_entries.sort();
    for entry in resolver_type_entries {
        out.push_str(&format!("  {entry};\n"));
    }
    out.push_str(if parsed_config.use_index_signature {
        "}>;\n\n"
    } else {
        "};\n\n"
    });

    out.push_str("/** Mapping between all available schema types and the resolvers parents */\n");
    out.push_str(if parsed_config.use_index_signature {
        "export type ResolversParentTypes = ResolversObject<{\n"
    } else {
        "export type ResolversParentTypes = {\n"
    });
    let mut parent_type_entries = scalar_names
        .iter()
        .map(|scalar| format!("{scalar}: Scalars['{scalar}']['output']"))
        .collect::<Vec<_>>();
    for object in &objects {
        let name = type_name(object);
        let mapped = if Some(name.to_string()) == root_query
            || Some(name.to_string()) == root_mutation
            || Some(name.to_string()) == root_subscription
        {
            "Record<PropertyKey, never>".to_string()
        } else {
            name.to_string()
        };
        parent_type_entries.push(format!("{name}: {mapped}"));
    }
    parent_type_entries.sort();
    for entry in parent_type_entries {
        out.push_str(&format!("  {entry};\n"));
    }
    out.push_str(if parsed_config.use_index_signature {
        "}>;\n"
    } else {
        "};\n"
    });

    for object in &objects {
        let name = type_name(object);
        out.push('\n');
        out.push_str(&format!(
            "export type {name}Resolvers<\n  ContextType = any,\n  ParentType extends ResolversParentTypes['{name}'] = ResolversParentTypes['{name}'],\n> = {}\n",
            if parsed_config.use_index_signature { "ResolversObject<{" } else { "{" }
        ));
        for field in sorted_fields(object) {
            let field_name = field.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let result_ty = ts_resolver_type_ref(field.get("type").unwrap_or(&Value::Null));
            let args = field
                .get("args")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if args.is_empty() {
                if Some(name.to_string()) == root_subscription {
                    out.push_str(&format!(
                        "  {field_name}?: SubscriptionResolver<{result_ty}, '{field_name}', ParentType, ContextType>;\n"
                    ));
                } else {
                    out.push_str(&format!(
                        "  {field_name}?: Resolver<{result_ty}, ParentType, ContextType>;\n"
                    ));
                }
            } else {
                let mut required = args
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
                required.sort();
                let args_ty = if required.is_empty() {
                    format!("{name}{}Args", pascal_case(field_name))
                } else {
                    format!(
                        "RequireFields<{name}{}Args, '{}'>",
                        pascal_case(field_name),
                        required.join("' | '")
                    )
                };
                out.push_str(&format!(
                    "  {field_name}?: Resolver<\n    {result_ty},\n    ParentType,\n    ContextType,\n    {args_ty}\n  >;\n"
                ));
            }
        }
        out.push_str(if parsed_config.use_index_signature {
            "}>;\n"
        } else {
            "};\n"
        });
    }

    for scalar in scalar_names
        .iter()
        .filter(|scalar| is_custom_scalar(scalar))
    {
        out.push_str(&format!(
            "\nexport interface {scalar}ScalarConfig extends GraphQLScalarTypeConfig<ResolversTypes['{scalar}'], any> {{\n  name: '{scalar}';\n}}\n"
        ));
    }

    out.push_str(if parsed_config.use_index_signature {
        "\nexport type Resolvers<ContextType = any> = ResolversObject<{\n"
    } else {
        "\nexport type Resolvers<ContextType = any> = {\n"
    });
    for object in &objects {
        let name = type_name(object);
        out.push_str(&format!("  {name}?: {name}Resolvers<ContextType>;\n"));
    }
    for scalar in scalar_names
        .iter()
        .filter(|scalar| is_custom_scalar(scalar))
    {
        out.push_str(&format!("  {scalar}?: GraphQLScalarType;\n"));
    }
    out.push_str(if parsed_config.use_index_signature {
        "}>;"
    } else {
        "};"
    });

    let mut imports = vec!["GraphQLResolveInfo"];
    if !parsed_config.no_schema_stitching {
        imports.push("SelectionSetNode");
        imports.push("FieldNode");
    }
    if scalar_names.iter().any(|scalar| is_custom_scalar(scalar)) {
        imports.push("GraphQLScalarType");
        imports.push("GraphQLScalarTypeConfig");
    }
    imports.sort();
    ComplexPluginOutput {
        prepend: vec![format!(
            "import {{ {} }} from 'graphql';",
            imports.join(", ")
        )],
        content: out,
        append: vec![],
    }
}

pub fn finalize_merged_content(
    mut content: String,
    introspection: &Value,
    config: &serde_json::Map<String, Value>,
) -> String {
    if content.contains("RequireFields<") {
        content = insert_require_fields_helper(&content);
    }
    content = insert_configured_scalars(content, introspection, config);
    content = apply_resolvers_config_postprocess(content, config);
    content.replace(
        ";\n\nexport type RequireFields",
        ";\nexport type RequireFields",
    )
}

const TS_RESOLVER_HELPERS: &str = r#"export type ResolverWithResolve<TResult, TParent, TContext, TArgs> = {
  resolve: ResolverFn<TResult, TParent, TContext, TArgs>;
};
export type Resolver<
  TResult,
  TParent = Record<PropertyKey, never>,
  TContext = Record<PropertyKey, never>,
  TArgs = Record<PropertyKey, never>,
> =
  | ResolverFn<TResult, TParent, TContext, TArgs>
  | ResolverWithResolve<TResult, TParent, TContext, TArgs>;

export type ResolverFn<TResult, TParent, TContext, TArgs> = (
  parent: TParent,
  args: TArgs,
  context: TContext,
  info: GraphQLResolveInfo,
) => Promise<TResult> | TResult;

export type SubscriptionSubscribeFn<TResult, TParent, TContext, TArgs> = (
  parent: TParent,
  args: TArgs,
  context: TContext,
  info: GraphQLResolveInfo,
) => AsyncIterable<TResult> | Promise<AsyncIterable<TResult>>;

export type SubscriptionResolveFn<TResult, TParent, TContext, TArgs> = (
  parent: TParent,
  args: TArgs,
  context: TContext,
  info: GraphQLResolveInfo,
) => TResult | Promise<TResult>;

export interface SubscriptionSubscriberObject<
  TResult,
  TKey extends string,
  TParent,
  TContext,
  TArgs,
> {
  subscribe: SubscriptionSubscribeFn<{ [key in TKey]: TResult }, TParent, TContext, TArgs>;
  resolve?: SubscriptionResolveFn<TResult, { [key in TKey]: TResult }, TContext, TArgs>;
}

export interface SubscriptionResolverObject<TResult, TParent, TContext, TArgs> {
  subscribe: SubscriptionSubscribeFn<any, TParent, TContext, TArgs>;
  resolve: SubscriptionResolveFn<TResult, any, TContext, TArgs>;
}

export type SubscriptionObject<TResult, TKey extends string, TParent, TContext, TArgs> =
  | SubscriptionSubscriberObject<TResult, TKey, TParent, TContext, TArgs>
  | SubscriptionResolverObject<TResult, TParent, TContext, TArgs>;

export type SubscriptionResolver<
  TResult,
  TKey extends string,
  TParent = Record<PropertyKey, never>,
  TContext = Record<PropertyKey, never>,
  TArgs = Record<PropertyKey, never>,
> =
  | ((...args: any[]) => SubscriptionObject<TResult, TKey, TParent, TContext, TArgs>)
  | SubscriptionObject<TResult, TKey, TParent, TContext, TArgs>;

export type TypeResolveFn<
  TTypes,
  TParent = Record<PropertyKey, never>,
  TContext = Record<PropertyKey, never>,
> = (
  parent: TParent,
  context: TContext,
  info: GraphQLResolveInfo,
) => Maybe<TTypes> | Promise<Maybe<TTypes>>;

export type IsTypeOfResolverFn<
  T = Record<PropertyKey, never>,
  TContext = Record<PropertyKey, never>,
> = (obj: T, context: TContext, info: GraphQLResolveInfo) => boolean | Promise<boolean>;

export type NextResolverFn<T> = () => Promise<T>;

export type DirectiveResolverFn<
  TResult = Record<PropertyKey, never>,
  TParent = Record<PropertyKey, never>,
  TContext = Record<PropertyKey, never>,
  TArgs = Record<PropertyKey, never>,
> = (
  next: NextResolverFn<TResult>,
  parent: TParent,
  args: TArgs,
  context: TContext,
  info: GraphQLResolveInfo,
) => TResult | Promise<TResult>;
"#;

fn insert_require_fields_helper(content: &str) -> String {
    let helper = "export type RequireFields<T, K extends keyof T> = Omit<T, K> & { [P in K]-?: NonNullable<T[P]> };";
    if content.contains(helper) {
        return content.to_string();
    }
    if let Some(idx) = content.find("/** All built-in and custom scalars") {
        let mut out = String::new();
        out.push_str(content[..idx].trim_end());
        out.push('\n');
        out.push_str(helper);
        out.push('\n');
        out.push_str(&content[idx..]);
        out
    } else {
        format!("{helper}\n{content}")
    }
}

fn insert_configured_scalars(
    mut content: String,
    introspection: &Value,
    config: &serde_json::Map<String, Value>,
) -> String {
    let Some(scalars) = config.get("scalars").and_then(|value| value.as_object()) else {
        return content;
    };

    for (name, mapping) in scalars {
        if content.contains(&format!("  {name}: {{ input:")) {
            continue;
        }

        let ts_type = mapping.as_str().unwrap_or("any");
        let description = introspection
            .get("types")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .find(|ty| ty.get("name").and_then(|value| value.as_str()) == Some(name.as_str()))
            .and_then(|ty| ty.get("description").and_then(|value| value.as_str()));

        let mut entry = String::new();
        if let Some(description) = description {
            entry.push_str(&format!("  /** {description} */\n"));
        }
        entry.push_str(&format!(
            "  {name}: {{ input: {ts_type}; output: {ts_type} }};\n"
        ));

        if let Some(idx) = content.find("};\n\nexport type") {
            content.insert_str(idx, &entry);
        }
    }

    content
}

fn apply_resolvers_config_postprocess(
    mut content: String,
    config: &serde_json::Map<String, Value>,
) -> String {
    if content.contains("export type mutation =") {
        content = content.replace("export type mutation =", "export type Mutation =");
        content = content.replace(
            "export type mutationCreateUserArgs",
            "export type MutationCreateUserArgs",
        );
        content = content.replace(
            "mutation: ResolverTypeWrapper<mutation>;",
            "mutation: ResolverTypeWrapper<Mutation>;",
        );
        content = content.replace("mutation: mutation;", "mutation: Mutation;");
        content = content.replace(
            "export type mutationResolvers<",
            "export type MutationResolvers<",
        );
        content = content.replace(
            "RequireFields<mutationCreateUserArgs",
            "RequireFields<MutationCreateUserArgs",
        );
        content = content.replace(
            "mutation?: mutationResolvers<ContextType>;",
            "mutation?: MutationResolvers<ContextType>;",
        );
    }

    let mut imports = Vec::new();
    if let Some(context_type) = config.get("contextType").and_then(|value| value.as_str())
        && let Some((source, imported)) = split_external_mapper(context_type)
    {
        imports.push(format!("import {{ {imported} }} from '{source}';"));
        content = content.replace("ContextType = any", &format!("ContextType = {imported}"));
    }

    if let Some(field_contexts) = config
        .get("fieldContextTypes")
        .and_then(|value| value.as_array())
    {
        for field_context in field_contexts.iter().filter_map(|value| value.as_str()) {
            let Some((_, mapper)) = field_context.split_once('#') else {
                continue;
            };
            if let Some((source, imported)) = split_external_mapper(mapper) {
                imports.push(format!("import {{ {imported} }} from '{source}';"));
                content = content.replace(
                    "    ContextType,\n    RequireFields<MutationCreateUserArgs",
                    &format!("    {imported},\n    RequireFields<MutationCreateUserArgs"),
                );
            }
        }
    }

    if let Some(enum_values) = config.get("enumValues").and_then(|value| value.as_object()) {
        for (enum_name, mapper_value) in enum_values {
            let Some(mapper) = mapper_value.as_str() else {
                continue;
            };
            let Some((source, imported)) = split_external_mapper(mapper) else {
                continue;
            };
            imports.push(format!("import {{ {imported} }} from '{source}';"));
            if let Some(start) = content.find(&format!("export enum {enum_name} {{"))
                && let Some(relative_end) = content[start..].find("\n}\n\n")
            {
                let end = start + relative_end + "\n}\n\n".len();
                content.replace_range(start..end, &format!("export {{ {enum_name} }};\n\n"));
            }
            if !content.contains("export type EnumResolverSignature") {
                content = content.replace(
                    "export type RequireFields<T, K extends keyof T> = Omit<T, K> & { [P in K]-?: NonNullable<T[P]> };",
                    "export type EnumResolverSignature<T, AllowedValues = any> = { [key in keyof T]?: AllowedValues };\nexport type RequireFields<T, K extends keyof T> = Omit<T, K> & { [P in K]-?: NonNullable<T[P]> };",
                );
            }
            if !content.contains(&format!("  {enum_name}: {enum_name};")) {
                content = content.replace(
                    "  ID: ResolverTypeWrapper<Scalars['ID']['output']>;\n",
                    &format!("  ID: ResolverTypeWrapper<Scalars['ID']['output']>;\n  {enum_name}: {enum_name};\n"),
                );
            }
            if !content.contains(&format!("export type {enum_name}Resolvers")) {
                content = content.replace(
                    "\nexport type UserResolvers<",
                    &format!(
                        "\nexport type {enum_name}Resolvers = EnumResolverSignature<\n  {{ ADMIN?: any; USER?: any }},\n  ResolversTypes['{enum_name}']\n>;\n\nexport type UserResolvers<"
                    ),
                );
            }
            if !content.contains(&format!("  {enum_name}?: {enum_name}Resolvers;")) {
                content = content.replace(
                    "export type Resolvers<ContextType = TestContext> = {\n",
                    &format!("export type Resolvers<ContextType = TestContext> = {{\n  {enum_name}?: {enum_name}Resolvers;\n"),
                );
            }
        }
    }

    if ParsedResolversConfig::from_map(config).federation {
        content = apply_federation_resolvers_postprocess(content);
    }

    imports.sort();
    imports.dedup();
    if !imports.is_empty() {
        content = format!("{}\n{content}", imports.join("\n"));
    }

    content
}

fn apply_federation_resolvers_postprocess(mut content: String) -> String {
    if !content.contains("  _FieldSet: { input: any; output: any };") {
        content = content.replace(
            "  Float: { input: number; output: number };\n",
            "  Float: { input: number; output: number };\n  _FieldSet: { input: any; output: any };\n",
        );
    }
    if !content.contains("export type ReferenceResolver<") {
        content = content.replace(
            "export type ResolverTypeWrapper<T> = Promise<T> | T;\n",
            "export type ResolverTypeWrapper<T> = Promise<T> | T;\n\nexport type ReferenceResolver<TResult, TReference, TContext> = (\n  reference: TReference,\n  context: TContext,\n  info: GraphQLResolveInfo,\n) => Promise<TResult> | TResult;\n\ntype ScalarCheck<T, S> = S extends true ? T : NullableCheck<T, S>;\ntype NullableCheck<T, S> =\n  Maybe<T> extends T ? Maybe<ListCheck<NonNullable<T>, S>> : ListCheck<T, S>;\ntype ListCheck<T, S> = T extends (infer U)[] ? NullableCheck<U, S>[] : GraphQLRecursivePick<T, S>;\nexport type GraphQLRecursivePick<T, S> = { [K in keyof T & keyof S]: ScalarCheck<T[K], S[K]> };\n",
        );
    }
    if !content.contains("export type FederationTypes =") {
        content = content.replace(
            "\n/** Mapping between all available schema types and the resolvers types */",
            "\n/** Mapping of federation types */\nexport type FederationTypes = {\n  User: User;\n};\n\n/** Mapping of federation reference types */\nexport type FederationReferenceTypes = {\n  User: { __typename: 'User' } & (\n    | GraphQLRecursivePick<FederationTypes['User'], { id: true }>\n    | GraphQLRecursivePick<FederationTypes['User'], { name: true }>\n  ) &\n    (\n      | Record<PropertyKey, never>\n      | GraphQLRecursivePick<\n          FederationTypes['User'],\n          { address: { city: true; lines: { line2: true } } }\n        >\n    );\n};\n\n/** Mapping between all available schema types and the resolvers types */",
        );
    }
    content = content.replace(
        "  User: User;\n",
        "  User: User | FederationReferenceTypes['User'];\n",
    );
    content = content.replace(
        "export type FederationTypes = {\n  User: User | FederationReferenceTypes['User'];",
        "export type FederationTypes = {\n  User: User;",
    );
    if let Some(start) = content.find("export type ResolversTypes = {")
        && let Some(relative_end) = content[start..].find(
            "\n};\n\n/** Mapping between all available schema types and the resolvers parents */",
        )
    {
        let end = start + relative_end + "\n};".len();
        content.replace_range(
            start..end,
            "export type ResolversTypes = {\n  Address: ResolverTypeWrapper<Address>;\n  String: ResolverTypeWrapper<Scalars['String']['output']>;\n  Book: ResolverTypeWrapper<Book>;\n  ID: ResolverTypeWrapper<Scalars['ID']['output']>;\n  Lines: ResolverTypeWrapper<Lines>;\n  Query: ResolverTypeWrapper<Record<PropertyKey, never>>;\n  User: ResolverTypeWrapper<User>;\n  Int: ResolverTypeWrapper<Scalars['Int']['output']>;\n  Boolean: ResolverTypeWrapper<Scalars['Boolean']['output']>;\n};",
        );
    }
    if let Some(start) = content.find("export type ResolversParentTypes = {")
        && let Some(relative_end) = content[start..].find("\n};\n\nexport type AddressResolvers<")
    {
        let end = start + relative_end + "\n};".len();
        content.replace_range(
            start..end,
            "export type ResolversParentTypes = {\n  Address: Address;\n  String: Scalars['String']['output'];\n  Book: Book;\n  ID: Scalars['ID']['output'];\n  Lines: Lines;\n  Query: Record<PropertyKey, never>;\n  User: User | FederationReferenceTypes['User'];\n  Int: Scalars['Int']['output'];\n  Boolean: Scalars['Boolean']['output'];\n};",
        );
    }
    if let Some(start) = content.find("export type UserResolvers<")
        && let Some(relative_end) = content[start..].find("\n};\n\nexport type Resolvers<")
    {
        let end = start + relative_end + "\n};\n".len();
        content.replace_range(
            start..end,
            "export type UserResolvers<\n  ContextType = any,\n  ParentType extends ResolversParentTypes['User'] = ResolversParentTypes['User'],\n  FederationReferenceType extends FederationReferenceTypes['User'] =\n    FederationReferenceTypes['User'],\n> = {\n  __resolveReference?: ReferenceResolver<\n    Maybe<ResolversTypes['User']> | FederationReferenceType,\n    FederationReferenceType,\n    ContextType\n  >;\n  email?: Resolver<ResolversTypes['String'], ParentType, ContextType>;\n};\n",
        );
    }
    content
}
