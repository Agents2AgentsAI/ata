use super::*;

fn format_definition_like(resp: Option<GotoDefinitionResponse>, not_found_msg: &str) -> String {
    match resp {
        None => not_found_msg.to_string(),
        Some(GotoDefinitionResponse::Scalar(loc)) => format_location(&loc),
        Some(GotoDefinitionResponse::Array(locs)) => {
            if locs.is_empty() {
                not_found_msg.to_string()
            } else {
                locs.iter()
                    .map(format_location)
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            if links.is_empty() {
                not_found_msg.to_string()
            } else {
                links
                    .iter()
                    .map(|link| {
                        format_uri_position(
                            link.target_uri.as_str(),
                            link.target_range.start.line,
                            link.target_range.start.character,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    }
}

pub(super) fn format_definition(resp: Option<GotoDefinitionResponse>) -> String {
    format_definition_like(resp, "No definition found.")
}

pub(super) fn format_implementation(resp: Option<GotoImplementationResponse>) -> String {
    format_definition_like(resp, "No implementation found.")
}

pub(super) fn format_references(refs: &[Location], limit: usize) -> String {
    if refs.is_empty() {
        "No references found.".to_string()
    } else {
        let total = refs.len();
        let shown = total.min(limit);
        let mut lines: Vec<String> = Vec::with_capacity(shown + 1);
        if total > limit {
            lines.push(format!(
                "Showing {shown} of {total} references (increase `limit` for more):"
            ));
        }
        for loc in refs.iter().take(limit) {
            lines.push(format_location(loc));
        }
        lines.join("\n")
    }
}

pub(super) fn format_uri_position(uri: &str, line0: u32, char0: u32) -> String {
    if let Ok(url) = url::Url::parse(uri)
        && let Ok(path) = url.to_file_path()
    {
        return format!("{}:{}:{}", path.display(), line0 + 1, char0 + 1);
    }
    format!("{uri}:{}:{}", line0 + 1, char0 + 1)
}

pub(super) fn format_location(loc: &Location) -> String {
    format_uri_position(
        loc.uri.as_str(),
        loc.range.start.line,
        loc.range.start.character,
    )
}

pub(super) fn format_hover(hover: Option<Hover>) -> String {
    match hover {
        None => "No hover information available.".to_string(),
        Some(hover) => match hover.contents {
            HoverContents::Scalar(content) => format_markup_content(content),
            HoverContents::Array(contents) => contents
                .into_iter()
                .map(format_markup_content)
                .collect::<Vec<_>>()
                .join("\n---\n"),
            HoverContents::Markup(markup) => markup.value,
        },
    }
}

fn format_markup_content(content: MarkedString) -> String {
    match content {
        MarkedString::String(text) => text,
        MarkedString::LanguageString(language_string) => {
            format!(
                "```{}\n{}\n```",
                language_string.language, language_string.value
            )
        }
    }
}

pub(super) fn format_document_symbols(resp: Option<DocumentSymbolResponse>) -> String {
    match resp {
        None => "No symbols found.".to_string(),
        Some(DocumentSymbolResponse::Flat(symbols)) => {
            if symbols.is_empty() {
                return "No symbols found.".to_string();
            }
            #[allow(deprecated)]
            symbols
                .iter()
                .map(|symbol| {
                    format!(
                        "{:?} {} [{}:{}]",
                        symbol.kind,
                        symbol.name,
                        symbol.location.range.start.line + 1,
                        symbol.location.range.start.character + 1
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        Some(DocumentSymbolResponse::Nested(symbols)) => {
            if symbols.is_empty() {
                return "No symbols found.".to_string();
            }
            let mut lines = Vec::new();
            format_nested_symbols(&symbols, 0, &mut lines);
            lines.join("\n")
        }
    }
}

fn format_nested_symbols(symbols: &[DocumentSymbol], depth: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    for symbol in symbols {
        out.push(format!(
            "{indent}{:?} {} [{}:{}]",
            symbol.kind,
            symbol.name,
            symbol.range.start.line + 1,
            symbol.range.start.character + 1
        ));
        if let Some(children) = &symbol.children {
            format_nested_symbols(children, depth + 1, out);
        }
    }
}

#[allow(deprecated)]
pub(super) fn format_workspace_symbols(
    symbols: &[SymbolInformation],
    limit: usize,
    total: usize,
    truncated: bool,
) -> String {
    if symbols.is_empty() {
        return "No symbols found.".to_string();
    }
    let shown = symbols.len().min(limit);
    let mut lines: Vec<String> = Vec::with_capacity(shown + 1);
    if truncated {
        lines.push(format!(
            "Showing {shown} of {total} symbols (increase `limit` for more):",
        ));
    }
    for symbol in symbols.iter().take(limit) {
        let pos = format_uri_position(
            symbol.location.uri.as_str(),
            symbol.location.range.start.line,
            symbol.location.range.start.character,
        );
        lines.push(format!("{:?} {} @ {}", symbol.kind, symbol.name, pos));
    }
    lines.join("\n")
}

pub(super) fn format_prepare_rename(resp: Option<PrepareRenameResponse>) -> String {
    let Some(resp) = resp else {
        return "No rename available.".to_string();
    };

    match resp {
        PrepareRenameResponse::Range(range) => format!(
            "Rename range: [{}:{}]-[{}:{}]",
            range.start.line + 1,
            range.start.character + 1,
            range.end.line + 1,
            range.end.character + 1
        ),
        PrepareRenameResponse::RangeWithPlaceholder { range, placeholder } => format!(
            "Rename range: [{}:{}]-[{}:{}]\nPlaceholder: {placeholder}",
            range.start.line + 1,
            range.start.character + 1,
            range.end.line + 1,
            range.end.character + 1
        ),
        PrepareRenameResponse::DefaultBehavior { default_behavior } => {
            format!("Rename supported (default behavior: {default_behavior}).")
        }
    }
}
