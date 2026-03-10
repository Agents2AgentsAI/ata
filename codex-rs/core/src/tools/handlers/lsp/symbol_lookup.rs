use super::*;

pub(super) fn symbol_name_matches(name: &str, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return false;
    }
    if name == query {
        return true;
    }
    if let Some(last) = query.rsplit("::").next() {
        return name == last;
    }
    false
}

pub(super) fn collect_document_symbol_candidates(
    resp: &Option<DocumentSymbolResponse>,
    query: &str,
    out: &mut Vec<(u32, u32, String, String)>,
) {
    let Some(resp) = resp else {
        return;
    };

    match resp {
        DocumentSymbolResponse::Flat(symbols) =>
        {
            #[allow(deprecated)]
            for symbol in symbols {
                if symbol_name_matches(&symbol.name, query) {
                    out.push((
                        symbol.location.range.start.line,
                        symbol.location.range.start.character,
                        symbol.name.clone(),
                        format!("{:?}", symbol.kind),
                    ));
                }
            }
        }
        DocumentSymbolResponse::Nested(symbols) => {
            collect_nested_doc_symbols(symbols, query, out);
        }
    }
}

fn collect_nested_doc_symbols(
    symbols: &[DocumentSymbol],
    query: &str,
    out: &mut Vec<(u32, u32, String, String)>,
) {
    for symbol in symbols {
        if symbol_name_matches(&symbol.name, query) {
            out.push((
                symbol.selection_range.start.line,
                symbol.selection_range.start.character,
                symbol.name.clone(),
                format!("{:?}", symbol.kind),
            ));
        }
        if let Some(children) = &symbol.children {
            collect_nested_doc_symbols(children, query, out);
        }
    }
}
