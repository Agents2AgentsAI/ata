use crate::queries::LanguageConfig;
use crate::queries::VariableRegexPattern;

const SYMBOLS_QUERY: &str = r#"
(function_declaration
  name: (identifier) @function.name) @function.def

(method_declaration
  name: (field_identifier) @method.name) @method.def

(type_declaration
  (type_spec
    name: (type_identifier) @struct.name
    type: (struct_type))) @struct.def

(type_declaration
  (type_spec
    name: (type_identifier) @interface.name
    type: (interface_type))) @interface.def

(type_declaration
  (type_spec
    name: (type_identifier) @type.name
    type: (_) @type.target)) @type.def
(#not-match? @type.def "^type\\s+\\w+\\s+struct\\b")
(#not-match? @type.def "^type\\s+\\w+\\s+interface\\b")

(const_declaration
  (const_spec
    name: (identifier) @const.name)) @const.def

(var_declaration
  (var_spec
    name: (identifier) @const.name)) @const.def
"#;

const CALLERS_QUERY: &str = r#"
(call_expression
  function: (identifier) @callee)

(call_expression
  function: (selector_expression
    field: (field_identifier) @callee))
"#;

const VARIABLES_QUERY: &str = r#"
(short_var_declaration
  left: (expression_list
    (identifier) @var.name))

(var_declaration
  (var_spec
    name: (identifier) @var.name))

(range_clause
  left: (expression_list
    (identifier) @var.name))

(parameter_declaration
  name: (identifier) @var.name)
"#;

const NON_CODE_QUERY: &str = r#"
(comment) @skip
(raw_string_literal) @skip
(interpreted_string_literal) @skip
"#;

const VARIABLE_REGEX_PATTERNS: &[VariableRegexPattern] = &[
    VariableRegexPattern {
        regex: r"(\w+)\s*:=",
        capture_group: 1,
    },
    VariableRegexPattern {
        regex: r"var\s+(\w+)",
        capture_group: 1,
    },
];

fn is_definition_line(line: &str, name: &str) -> bool {
    line.contains(&format!("func {name}"))
}

fn is_test_symbol(name: &str, file: &str) -> bool {
    name.starts_with("Test") || file.ends_with("_test.go")
}

fn variable_name_filter(name: &str) -> bool {
    !name.is_empty() && name != "_"
}

pub fn config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_go::LANGUAGE.into(),
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
