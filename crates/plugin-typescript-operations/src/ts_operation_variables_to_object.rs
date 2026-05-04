//! Port of `packages/plugins/typescript/operations/src/ts-operation-variables-to-object.ts` (minimal).

use graphql_parser::query::{OperationDefinition, Type as AstType};

use crate::visitor::{TypeRef, parse_ast_type_ref};

pub struct TypeScriptOperationVariablesToObject<'a> {
    pub is_enum: &'a dyn Fn(&str) -> bool,
    pub is_scalar: &'a dyn Fn(&str) -> bool,
    pub immutable_types: bool,
    pub type_prefix: &'a str,
}

impl<'a> TypeScriptOperationVariablesToObject<'a> {
    pub fn new(
        is_enum: &'a dyn Fn(&str) -> bool,
        is_scalar: &'a dyn Fn(&str) -> bool,
        immutable_types: bool,
        type_prefix: &'a str,
    ) -> Self {
        Self {
            is_enum,
            is_scalar,
            immutable_types,
            type_prefix,
        }
    }

    fn list_wrapper(&self) -> &'static str {
        if self.immutable_types {
            "ReadonlyArray"
        } else {
            "Array"
        }
    }

    fn scalar_input_ts(name: &str) -> String {
        format!("Scalars['{name}']['input']")
    }

    fn qualify_type_name(&self, name: &str) -> String {
        format!("{}{name}", self.type_prefix)
    }

    fn qualified_scalar_input_ts(&self, name: &str) -> String {
        format!("{}{}", self.type_prefix, Self::scalar_input_ts(name))
    }

    fn input_maybe(&self, inner: impl AsRef<str>) -> String {
        format!("{}InputMaybe<{}>", self.type_prefix, inner.as_ref())
    }

    fn exact(&self, inner: impl AsRef<str>) -> String {
        format!("{}Exact<{}>", self.type_prefix, inner.as_ref())
    }

    fn input_ts(&self, type_ref: &TypeRef) -> String {
        match type_ref {
            TypeRef::NonNull(inner) => self.input_ts(inner),
            TypeRef::List(inner) => {
                format!("{}<{}>", self.list_wrapper(), self.input_ts(inner))
            }
            TypeRef::Named(name) => {
                if (self.is_enum)(name) {
                    self.input_maybe(self.qualify_type_name(name))
                } else if (self.is_scalar)(name) {
                    self.input_maybe(self.qualified_scalar_input_ts(name))
                } else {
                    self.input_maybe(self.qualify_type_name(name))
                }
            }
        }
    }

    fn input_field(&self, type_ref: &TypeRef) -> (bool, String) {
        let optional = !matches!(type_ref, TypeRef::NonNull(_));
        let ts = match type_ref {
            TypeRef::NonNull(inner) => match inner.as_ref() {
                TypeRef::List(l) => {
                    format!("{}<{}>", self.list_wrapper(), self.input_ts(l))
                }
                TypeRef::Named(name) => {
                    if (self.is_enum)(name) {
                        self.qualify_type_name(name)
                    } else if (self.is_scalar)(name) {
                        self.qualified_scalar_input_ts(name)
                    } else {
                        self.qualify_type_name(name)
                    }
                }
                TypeRef::NonNull(_) => self.input_ts(inner),
            },
            TypeRef::List(inner) => {
                self.input_maybe(format!("{}<{}>", self.list_wrapper(), self.input_ts(inner)))
            }
            TypeRef::Named(name) => {
                if (self.is_enum)(name) {
                    self.input_maybe(self.qualify_type_name(name))
                } else if (self.is_scalar)(name) {
                    self.input_maybe(self.qualified_scalar_input_ts(name))
                } else {
                    self.input_maybe(self.qualify_type_name(name))
                }
            }
        };
        (optional, ts)
    }

    pub fn transform_operation_variables(
        &self,
        op: &OperationDefinition<'static, String>,
        avoid_optionals: bool,
    ) -> String {
        let mut vars: Vec<(String, (bool, String))> = Vec::new();

        let push_vars = |vars: &mut Vec<(String, (bool, String))>,
                         vname: &str,
                         vtype: &AstType<String>,
                         this: &Self| {
            let tr = parse_ast_type_ref(vtype);
            let (opt, ts) = this.input_field(&tr);
            vars.push((vname.to_string(), (opt, ts)));
        };

        match op {
            OperationDefinition::Query(q) => {
                for v in &q.variable_definitions {
                    push_vars(&mut vars, &v.name, &v.var_type, self);
                }
            }
            OperationDefinition::Mutation(m) => {
                for v in &m.variable_definitions {
                    push_vars(&mut vars, &v.name, &v.var_type, self);
                }
            }
            OperationDefinition::Subscription(s) => {
                for v in &s.variable_definitions {
                    push_vars(&mut vars, &v.name, &v.var_type, self);
                }
            }
            OperationDefinition::SelectionSet(_) => {}
        }

        if vars.is_empty() {
            return self.exact("{ [key: string]: never }");
        }

        let mut inner = String::new();
        inner.push_str("{\n");
        for (name, (opt, ts)) in vars {
            let q = if opt && !avoid_optionals { "?" } else { "" };
            inner.push_str(&format!("  {name}{q}: {ts};\n"));
        }
        inner.push('}');
        self.exact(inner)
    }
}
