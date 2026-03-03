use crate::queries::LanguageConfig;

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

pub fn config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_python::LANGUAGE.into(),
        symbols_query: SYMBOLS_QUERY,
        callers_query: CALLERS_QUERY,
        variables_query: VARIABLES_QUERY,
        non_code_query: NON_CODE_QUERY,
    }
}
