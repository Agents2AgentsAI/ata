use crate::queries::LanguageConfig;

const SYMBOLS_QUERY: &str = r#"
(function_declaration
  name: (identifier) @function.name) @function.def

(class_declaration
  name: (type_identifier) @class.name) @class.def

(method_definition
  name: (property_identifier) @method.name) @method.def

(lexical_declaration
  (variable_declarator
    name: (identifier) @const.name
    value: (arrow_function))) @const.def

(interface_declaration
  name: (type_identifier) @interface.name) @interface.def

(type_alias_declaration
  name: (type_identifier) @type.name) @type.def

(enum_declaration
  name: (identifier) @enum.name) @enum.def
"#;

const CALLERS_QUERY: &str = r#"
(call_expression
  function: (identifier) @callee)

(call_expression
  function: (member_expression
    property: (property_identifier) @callee))
"#;

const VARIABLES_QUERY: &str = r#"
(variable_declarator
  name: (identifier) @var.name)

(variable_declarator
  name: (object_pattern
    (shorthand_property_identifier_pattern) @var.name))

(variable_declarator
  name: (array_pattern
    (identifier) @var.name))

(for_in_statement
  left: (identifier) @var.name)

(for_in_statement
  left: (lexical_declaration
    (variable_declarator
      name: (identifier) @var.name)))

(required_parameter
  pattern: (identifier) @var.name)

(optional_parameter
  pattern: (identifier) @var.name)
"#;

const NON_CODE_QUERY: &str = r#"
(comment) @skip
(string) @skip
(template_string) @skip
"#;

const JS_SYMBOLS_QUERY: &str = r#"
(function_declaration
  name: (identifier) @function.name) @function.def

(class_declaration
  name: (identifier) @class.name) @class.def

(method_definition
  name: (property_identifier) @method.name) @method.def

(lexical_declaration
  (variable_declarator
    name: (identifier) @const.name
    value: (arrow_function))) @const.def
"#;

const JS_CALLERS_QUERY: &str = CALLERS_QUERY;

const JS_NON_CODE_QUERY: &str = NON_CODE_QUERY;

const JS_VARIABLES_QUERY: &str = r#"
(variable_declarator
  name: (identifier) @var.name)

(variable_declarator
  name: (object_pattern
    (shorthand_property_identifier_pattern) @var.name))

(variable_declarator
  name: (array_pattern
    (identifier) @var.name))

(for_in_statement
  left: (identifier) @var.name)

(formal_parameters
  (identifier) @var.name)
"#;

pub fn config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        symbols_query: SYMBOLS_QUERY,
        callers_query: CALLERS_QUERY,
        variables_query: VARIABLES_QUERY,
        non_code_query: NON_CODE_QUERY,
    }
}

pub fn javascript_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_javascript::LANGUAGE.into(),
        symbols_query: JS_SYMBOLS_QUERY,
        callers_query: JS_CALLERS_QUERY,
        variables_query: JS_VARIABLES_QUERY,
        non_code_query: JS_NON_CODE_QUERY,
    }
}
