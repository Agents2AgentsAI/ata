use crate::queries::LanguageConfig;
use crate::queries::VariableRegexPattern;

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

const VARIABLE_REGEX_PATTERNS: &[VariableRegexPattern] = &[VariableRegexPattern {
    regex: r"\b(?:int|long|float|double|boolean|char|byte|short|String|var|final\s+\w+)\s+(\w+)\s*[=;,)]",
    capture_group: 1,
}];

fn is_definition_line(line: &str, name: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
        return false;
    }

    trimmed.contains(&format!("class {name}"))
        || trimmed.contains(&format!("interface {name}"))
        || trimmed.contains(&format!("enum {name}"))
        || (trimmed.contains(&format!("{name}("))
            && (trimmed.contains("void ")
                || trimmed.contains("int ")
                || trimmed.contains("String ")
                || trimmed.contains("boolean ")
                || trimmed.contains("long ")
                || trimmed.contains("double ")
                || trimmed.contains("float ")
                || trimmed.contains("char ")
                || trimmed.contains("byte ")
                || trimmed.contains("short ")
                || trimmed.contains("public ")
                || trimmed.contains("private ")
                || trimmed.contains("protected ")
                || trimmed.contains("static ")
                || trimmed.contains("final ")
                || trimmed.contains("abstract ")
                || trimmed.contains("default ")
                || trimmed.contains("synchronized ")
                || trimmed.contains("native ")))
}

fn is_test_symbol(_name: &str, file: &str) -> bool {
    file.contains("Test") || file.contains("/test/")
}

fn variable_name_filter(name: &str) -> bool {
    !name.is_empty() && name != "_"
}

pub fn config() -> LanguageConfig {
    LanguageConfig {
        language: tree_sitter_java::LANGUAGE.into(),
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
