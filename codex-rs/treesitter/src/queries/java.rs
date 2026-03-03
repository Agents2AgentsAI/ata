use crate::queries::LanguageConfig;

const SYMBOLS_QUERY: &str = r#"
(class_declaration
  name: (identifier) @class.name) @class.def

(interface_declaration
  name: (identifier) @interface.name) @interface.def

(enum_declaration
  name: (identifier) @enum.name) @enum.def

(record_declaration
  name: (identifier) @class.name) @class.def

(method_declaration
  name: (identifier) @method.name) @method.def

(constructor_declaration
  name: (identifier) @method.name) @method.def
"#;

const CALLERS_QUERY: &str = r#"
(method_invocation
  name: (identifier) @callee)

(object_creation_expression
  type: (type_identifier) @callee)
"#;

const VARIABLES_QUERY: &str = r#"
(local_variable_declaration
  declarator: (variable_declarator
    name: (identifier) @var.name))

(enhanced_for_statement
  name: (identifier) @var.name)

(formal_parameter
  name: (identifier) @var.name)
"#;

const NON_CODE_QUERY: &str = r#"
(line_comment) @skip
(block_comment) @skip
(string_literal) @skip
"#;

pub fn config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_java::LANGUAGE.into(),
        symbols_query: SYMBOLS_QUERY,
        callers_query: CALLERS_QUERY,
        variables_query: VARIABLES_QUERY,
        non_code_query: NON_CODE_QUERY,
    }
}
