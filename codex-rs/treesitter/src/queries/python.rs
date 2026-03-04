use crate::queries::LanguageConfig;
use crate::queries::VariableRegexPattern;

const SYMBOLS_QUERY: &str = r#"
(module
  (function_definition
    name: (identifier) @function.name) @function.def)

(class_definition
  name: (identifier) @class.name) @class.def

(class_definition
  name: (identifier) @method.parent
  body: (block
    (function_definition
      name: (identifier) @method.name) @method.def))
"#;

const CALLERS_QUERY: &str = r#"
(call
  function: (identifier) @callee)

(call
  function: (attribute
    object: (_) @qualifier
    attribute: (identifier) @callee))
"#;

const VARIABLES_QUERY: &str = r#"
(assignment
  left: (identifier) @var.name)

(assignment
  left: (pattern_list
    (identifier) @var.name))

(assignment
  left: (tuple_pattern
    (identifier) @var.name))

(for_statement
  left: (identifier) @var.name)

(for_statement
  left: (tuple_pattern
    (identifier) @var.name))

(with_item
  (as_pattern
    alias: (as_pattern_target
      (identifier) @var.name)))

(parameters
  (identifier) @var.name)

(parameters
  (default_parameter
    name: (identifier) @var.name))

(parameters
  (typed_parameter
    (identifier) @var.name))

(parameters
  (typed_default_parameter
    name: (identifier) @var.name))
"#;

const NON_CODE_QUERY: &str = r#"
(comment) @skip
(string) @skip
"#;

const VARIABLE_REGEX_PATTERNS: &[VariableRegexPattern] = &[VariableRegexPattern {
    regex: r"(?m)^\s+(\w+)\s*=",
    capture_group: 1,
}];

fn is_definition_line(line: &str, name: &str) -> bool {
    line.contains(&format!("def {name}"))
}

fn is_test_symbol(name: &str, file: &str) -> bool {
    name.starts_with("test_") || file.contains("test_") || file.contains("_test.")
}

fn variable_name_filter(name: &str) -> bool {
    !name.is_empty() && name != "_" && name != "self" && !name.starts_with('_')
}

pub fn config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_python::LANGUAGE.into(),
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
