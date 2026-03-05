use crate::queries::LanguageConfig;
use crate::queries::VariableRegexPattern;

const SYMBOLS_QUERY: &str = r#"
(class_definition
  name: (identifier) @class.name) @class.def

(object_definition
  name: (identifier) @class.name) @class.def

(trait_definition
  name: (identifier) @trait.name) @trait.def

(function_definition
  name: (identifier) @function.name) @function.def

(type_definition
  name: (type_identifier) @type.name) @type.def
"#;

const CALLERS_QUERY: &str = r#"
(call_expression
  function: (identifier) @callee)

(call_expression
  function: (field_expression
    value: (_) @qualifier
    field: (identifier) @callee))
"#;

const VARIABLES_QUERY: &str = r#"
(val_definition
  pattern: (identifier) @var.name)

(var_definition
  pattern: (identifier) @var.name)

(parameter
  name: (identifier) @var.name)
"#;

const NON_CODE_QUERY: &str = r#"
(comment) @skip
(block_comment) @skip
(string) @skip
(interpolated_string_expression) @skip
"#;

const VARIABLE_REGEX_PATTERNS: &[VariableRegexPattern] = &[VariableRegexPattern {
    regex: r"\b(?:val|var)\s+(\w+)",
    capture_group: 1,
}];

fn is_definition_line(line: &str, name: &str) -> bool {
    line.contains(&format!("def {name}"))
        || line.contains(&format!("object {name}"))
        || line.contains(&format!("class {name}"))
        || line.contains(&format!("trait {name}"))
}

fn is_test_symbol(name: &str, file: &str) -> bool {
    let in_test_location =
        file.contains("Spec") || file.contains("Test") || file.contains("/test/");
    if in_test_location {
        return name.starts_with("test")
            || name.starts_with("Test")
            || name.starts_with("should")
            || name == "spec";
    }
    false
}

fn variable_name_filter(name: &str) -> bool {
    !name.is_empty() && name != "_"
}

pub fn config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_scala::LANGUAGE.into(),
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
