use crate::queries::LanguageConfig;
use crate::queries::VariableRegexPattern;

const SYMBOLS_QUERY: &str = r#"
(source_file
  (function_item
    name: (identifier) @function.name) @function.def)

(mod_item
  body: (declaration_list
    (function_item
      name: (identifier) @function.name) @function.def))

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
    value: (_) @qualifier
    field: (field_identifier) @callee))

(call_expression
  function: (scoped_identifier
    path: (_) @qualifier
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

const NON_CODE_QUERY: &str = r#"
(line_comment) @skip
(block_comment) @skip
(string_literal) @skip
(raw_string_literal) @skip
"#;

const VARIABLE_REGEX_PATTERNS: &[VariableRegexPattern] = &[VariableRegexPattern {
    regex: r"let\s+(?:mut\s+)?(\w+)",
    capture_group: 1,
}];

fn is_definition_line(line: &str, name: &str) -> bool {
    line.contains(&format!("fn {name}"))
}

fn is_test_symbol(name: &str, file: &str) -> bool {
    name.starts_with("test_")
        || file.contains("/tests/")
        || file.contains("\\tests\\")
        || file.ends_with("_test.rs")
}

fn variable_name_filter(name: &str) -> bool {
    !name.is_empty() && name != "_" && name != "self"
}

pub fn config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_rust::LANGUAGE.into(),
        symbols_query: SYMBOLS_QUERY,
        callers_query: CALLERS_QUERY,
        variables_query: VARIABLES_QUERY,
        non_code_query: NON_CODE_QUERY,
        definition_matcher: is_definition_line,
        test_symbol_matcher: is_test_symbol,
        variable_regex_patterns: VARIABLE_REGEX_PATTERNS,
        variable_name_filter,
    }
}
