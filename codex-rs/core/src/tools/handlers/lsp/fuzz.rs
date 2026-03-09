use super::*;

pub(super) fn is_empty_definition(resp: &Option<GotoDefinitionResponse>) -> bool {
    match resp {
        None => true,
        Some(GotoDefinitionResponse::Scalar(_)) => false,
        Some(GotoDefinitionResponse::Array(locs)) => locs.is_empty(),
        Some(GotoDefinitionResponse::Link(links)) => links.is_empty(),
    }
}

pub(super) async fn fuzz_query<T, Q, Fut, E>(
    fuzz: bool,
    line: u32,
    character: u32,
    max_attempts: usize,
    mut query: Q,
    mut is_empty: E,
) -> (T, Option<(u32, u32)>)
where
    Q: FnMut(u32, u32) -> Fut,
    Fut: Future<Output = T>,
    E: FnMut(&T) -> bool,
{
    let mut result = query(line, character).await;
    let mut resolved = None;
    if fuzz && is_empty(&result) {
        for (candidate_line, candidate_character) in fuzz_positions(line, character)
            .into_iter()
            .skip(1)
            .take(max_attempts)
        {
            let candidate = query(candidate_line, candidate_character).await;
            if !is_empty(&candidate) {
                result = candidate;
                resolved = Some((candidate_line, candidate_character));
                break;
            }
        }
    }
    (result, resolved)
}

pub(super) fn resolved_at_prefix(path: &Path, resolved: Option<(u32, u32)>) -> String {
    let mut out = String::new();
    if let Some((line, character)) = resolved {
        let _ = writeln!(
            out,
            "Resolved at {}:{}:{}",
            path.display(),
            line + 1,
            character + 1
        );
    }
    out
}

fn fuzz_positions(line0: u32, char0: u32) -> Vec<(u32, u32)> {
    const LINE_OFFSETS: [i32; 5] = [0, -1, 1, -2, 2];
    const CHAR_OFFSETS: [i32; 17] = [0, -1, 1, -2, 2, -3, 3, -4, 4, -5, 5, -6, 6, -7, 7, -8, 8];

    let mut out = Vec::with_capacity(LINE_OFFSETS.len() * CHAR_OFFSETS.len());
    let mut seen = std::collections::HashSet::new();
    for line_offset in LINE_OFFSETS {
        for char_offset in CHAR_OFFSETS {
            let line = line0 as i32 + line_offset;
            let character = char0 as i32 + char_offset;
            if line < 0 || character < 0 {
                continue;
            }
            let pos = (line as u32, character as u32);
            if seen.insert(pos) {
                out.push(pos);
            }
        }
    }
    out
}
