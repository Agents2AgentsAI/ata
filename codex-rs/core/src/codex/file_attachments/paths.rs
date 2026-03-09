use super::*;

pub(crate) fn inject_local_pdf_paths_from_text_inputs(
    inputs: &mut Vec<UserInput>,
    cwd: &Path,
    sandbox_policy: &SandboxPolicy,
) {
    let mut seen_paths = HashSet::new();
    for input in inputs.iter() {
        match input {
            UserInput::LocalFile { path } => {
                seen_paths.insert(canonical_or_path(path));
            }
            UserInput::UploadedFile { source_path, .. } => {
                seen_paths.insert(canonical_or_path(source_path));
            }
            _ => {}
        }
    }

    let discovered = extract_local_pdf_paths_from_text_inputs(inputs, cwd, sandbox_policy);
    for path in discovered {
        if seen_paths.insert(path.clone()) {
            inputs.push(UserInput::LocalFile { path });
        }
    }
}

fn extract_local_pdf_paths_from_text_inputs(
    inputs: &[UserInput],
    cwd: &Path,
    sandbox_policy: &SandboxPolicy,
) -> Vec<PathBuf> {
    let mut discovered = Vec::new();
    let mut seen = HashSet::new();
    for input in inputs {
        let UserInput::Text { text, .. } = input else {
            continue;
        };
        for token in split_text_path_tokens(text) {
            let candidate = strip_wrapping_quote_pair(token);
            if !is_pdf_path_token(candidate) || looks_like_url(candidate) {
                continue;
            }
            let resolved = resolve_candidate_path(candidate, cwd);
            let Ok(canonical) = std::fs::canonicalize(&resolved) else {
                continue;
            };
            if !canonical.is_file()
                || !is_canonical_path_within_allowed_roots(&canonical, cwd, sandbox_policy)
            {
                continue;
            }
            if seen.insert(canonical.clone()) {
                discovered.push(canonical);
            }
        }
    }
    discovered
}

fn split_text_path_tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();

    while let Some((start, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }

        if ch == '"' || ch == '\'' {
            let quote = ch;
            let mut token_end = text.len();
            let mut found_closing = false;
            for (idx, next_ch) in chars.by_ref() {
                if next_ch == quote {
                    token_end = idx + next_ch.len_utf8();
                    found_closing = true;
                    break;
                }
            }

            if found_closing
                && chars
                    .peek()
                    .is_none_or(|(_, trailing)| trailing.is_whitespace())
            {
                tokens.push(&text[start..token_end]);
                continue;
            }

            while let Some((idx, next_ch)) = chars.peek().copied() {
                if next_ch.is_whitespace() {
                    token_end = idx;
                    break;
                }
                token_end = idx + next_ch.len_utf8();
                let _ = chars.next();
            }
            tokens.push(&text[start..token_end]);
            continue;
        }

        let mut token_end = text.len();
        while let Some((idx, next_ch)) = chars.peek().copied() {
            if next_ch.is_whitespace() {
                token_end = idx;
                break;
            }
            token_end = idx + next_ch.len_utf8();
            let _ = chars.next();
        }
        tokens.push(&text[start..token_end]);
    }

    tokens
}

fn strip_wrapping_quote_pair(token: &str) -> &str {
    if token.len() >= 2
        && ((token.starts_with('"') && token.ends_with('"'))
            || (token.starts_with('\'') && token.ends_with('\'')))
    {
        return &token[1..token.len() - 1];
    }
    token
}

fn is_pdf_path_token(token: &str) -> bool {
    !token.is_empty() && token.to_ascii_lowercase().ends_with(".pdf")
}

fn looks_like_url(token: &str) -> bool {
    token.contains("://")
}

fn resolve_candidate_path(token: &str, cwd: &Path) -> PathBuf {
    let path = PathBuf::from(token);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

pub(super) fn canonical_or_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn is_canonical_path_within_allowed_roots(
    canonical_path: &Path,
    cwd: &Path,
    sandbox_policy: &SandboxPolicy,
) -> bool {
    let canonical_cwd = canonical_or_path(cwd);
    if canonical_path.starts_with(&canonical_cwd) {
        return true;
    }

    sandbox_policy
        .get_writable_roots_with_cwd(cwd)
        .iter()
        .any(|root| {
            if root.is_path_writable(canonical_path) {
                return true;
            }

            let canonical_root = canonical_or_path(root.root.as_path());
            if !canonical_path.starts_with(&canonical_root) {
                return false;
            }

            for read_only_subpath in &root.read_only_subpaths {
                let canonical_subpath = canonical_or_path(read_only_subpath.as_path());
                if canonical_path.starts_with(&canonical_subpath) {
                    return false;
                }
            }
            true
        })
}
