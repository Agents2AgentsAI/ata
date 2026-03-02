use crate::queries::LanguageConfig;

const SYMBOLS_QUERY: &str = r#"
(function_item
  name: (identifier) @function.name) @function.def

(impl_item
  type: (_) @impl.type
  body: (declaration_list
    (function_item
      name: (identifier) @method.name) @method.def))

(struct_item
  name: (type_identifier) @struct.name) @struct.def

(enum_item
  name: (type_identifier) @enum.name) @enum.def

(trait_item
  name: (type_identifier) @trait.name) @trait.def

(type_item
  name: (type_identifier) @type.name) @type.def

(const_item
  name: (identifier) @const.name) @const.def

(static_item
  name: (identifier) @const.name) @const.def

(mod_item
  name: (identifier) @mod.name) @mod.def
"#;

const CALLERS_QUERY: &str = r#"
(call_expression
  function: (identifier) @callee)

(call_expression
  function: (field_expression
    field: (field_identifier) @callee))

(call_expression
  function: (scoped_identifier
    name: (identifier) @callee))

(macro_invocation
  macro: (identifier) @callee)
"#;

const VARIABLES_QUERY: &str = r#"
(let_declaration
  pattern: (identifier) @var.name)

(let_declaration
  pattern: (tuple_pattern
    (identifier) @var.name))

(let_declaration
  pattern: (tuple_struct_pattern
    (identifier) @var.name))

(for_expression
  pattern: (identifier) @var.name)

(if_let_expression
  pattern: (_
    (identifier) @var.name))

(parameter
  pattern: (identifier) @var.name)
"#;

pub fn config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_rust::LANGUAGE.into(),
        symbols_query: SYMBOLS_QUERY,
        callers_query: CALLERS_QUERY,
        variables_query: VARIABLES_QUERY,
    }
}
