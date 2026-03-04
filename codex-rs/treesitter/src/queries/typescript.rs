use crate::queries::LanguageConfig;
use crate::queries::VariableRegexPattern;

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
    object: (_) @qualifier
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

const TS_VARIABLE_REGEX_PATTERNS: &[VariableRegexPattern] = &[VariableRegexPattern {
    regex: r"(?:let|const|var)\s+(\w+)",
    capture_group: 1,
}];

const JS_VARIABLE_REGEX_PATTERNS: &[VariableRegexPattern] = &[VariableRegexPattern {
    regex: r"(?:let|const|var)\s+(\w+)",
    capture_group: 1,
}];

fn ts_definition_line(line: &str, name: &str) -> bool {
    line.contains(&format!("function {name}")) || line.contains(&format!("{name} ="))
}

fn js_definition_line(line: &str, name: &str) -> bool {
    line.contains(&format!("function {name}")) || line.contains(&format!("{name} ="))
}

fn is_test_symbol(name: &str, file: &str) -> bool {
    let in_test_file =
        file.contains(".test.") || file.contains(".spec.") || file.contains("__tests__");
    // In test files, only mark symbols that look like test constructs or
    // follow common test-function naming patterns.  This avoids classifying
    // helper utilities (e.g. `createServer`) as tests.
    if in_test_file {
        return name.starts_with("test")
            || name.starts_with("Test")
            || name == "describe"
            || name == "it"
            || name == "beforeEach"
            || name == "afterEach"
            || name == "beforeAll"
            || name == "afterAll";
    }
    false
}

fn variable_name_filter(name: &str) -> bool {
    !name.is_empty() && name != "_"
}

pub fn config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        symbols_query: SYMBOLS_QUERY,
        callers_query: CALLERS_QUERY,
        variables_query: VARIABLES_QUERY,
        non_code_query: NON_CODE_QUERY,
        definition_matcher: ts_definition_line,
        test_symbol_matcher: is_test_symbol,
        variable_regex_patterns: TS_VARIABLE_REGEX_PATTERNS,
        variable_name_filter,
    }
}

pub fn javascript_config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_javascript::LANGUAGE.into(),
        symbols_query: JS_SYMBOLS_QUERY,
        callers_query: JS_CALLERS_QUERY,
        variables_query: JS_VARIABLES_QUERY,
        non_code_query: JS_NON_CODE_QUERY,
        definition_matcher: js_definition_line,
        test_symbol_matcher: is_test_symbol,
        variable_regex_patterns: JS_VARIABLE_REGEX_PATTERNS,
        variable_name_filter,
    }
}
