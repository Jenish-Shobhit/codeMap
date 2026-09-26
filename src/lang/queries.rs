//! Tags-style queries per language: `@def.<kind>` + `@name` for definitions,
//! `@ref.call` + `@name` (+ `@recv`) for calls.

use super::Lang;

const PYTHON: &str = r#"
(function_definition name: (identifier) @name) @def.function
(class_definition name: (identifier) @name) @def.class
(call function: (identifier) @name) @ref.call
(call function: (attribute object: (_) @recv attribute: (identifier) @name)) @ref.call
"#;

const RUST: &str = r#"
(function_item name: (identifier) @name) @def.function
(function_signature_item name: (identifier) @name) @def.function
(struct_item name: (type_identifier) @name) @def.struct
(union_item name: (type_identifier) @name) @def.struct
(enum_item name: (type_identifier) @name) @def.enum
(trait_item name: (type_identifier) @name) @def.trait
(type_item name: (type_identifier) @name) @def.type
(impl_item type: (_) @name) @def.impl
(mod_item name: (identifier) @name) @def.module
(call_expression function: (identifier) @name) @ref.call
(call_expression function: (scoped_identifier path: (_) @recv name: (identifier) @name)) @ref.call
(call_expression function: (field_expression value: (_) @recv field: (field_identifier) @name)) @ref.call
(call_expression function: (generic_function function: (identifier) @name)) @ref.call
(call_expression function: (generic_function function: (scoped_identifier path: (_) @recv name: (identifier) @name))) @ref.call
(call_expression function: (generic_function function: (field_expression value: (_) @recv field: (field_identifier) @name))) @ref.call
"#;

const JAVASCRIPT: &str = r#"
(function_declaration name: (identifier) @name) @def.function
(generator_function_declaration name: (identifier) @name) @def.function
(class_declaration name: (_) @name) @def.class
(method_definition name: (_) @name) @def.method
(variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)]) @def.function
(call_expression function: (identifier) @name) @ref.call
(call_expression function: (member_expression object: (_) @recv property: (property_identifier) @name)) @ref.call
(new_expression constructor: (identifier) @name) @ref.call
"#;

const TYPESCRIPT_EXTRA: &str = r#"
(abstract_class_declaration name: (_) @name) @def.class
(interface_declaration name: (_) @name) @def.interface
(type_alias_declaration name: (_) @name) @def.type
(enum_declaration name: (_) @name) @def.enum
"#;

const GO: &str = r#"
(function_declaration name: (identifier) @name) @def.function
(method_declaration
  receiver: (parameter_list
    (parameter_declaration
      type: [(type_identifier) @recv
             (pointer_type (type_identifier) @recv)
             (generic_type type: (type_identifier) @recv)
             (pointer_type (generic_type type: (type_identifier) @recv))]))
  name: (field_identifier) @name) @def.method
(type_spec name: (type_identifier) @name) @def.struct
(call_expression function: (identifier) @name) @ref.call
(call_expression function: (selector_expression operand: (_) @recv field: (field_identifier) @name)) @ref.call
"#;

pub fn source(lang: Lang) -> &'static str {
    use std::sync::OnceLock;
    static TS: OnceLock<String> = OnceLock::new();
    match lang {
        Lang::Python => PYTHON,
        Lang::Rust => RUST,
        Lang::JavaScript => JAVASCRIPT,
        Lang::TypeScript | Lang::Tsx => {
            TS.get_or_init(|| format!("{JAVASCRIPT}\n{TYPESCRIPT_EXTRA}"))
        }
        Lang::Go => GO,
    }
}
