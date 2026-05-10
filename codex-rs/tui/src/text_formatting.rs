use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

pub(crate) fn capitalize_first(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(first) => {
            let mut capitalized = first.to_uppercase().collect::<String>();
            capitalized.push_str(chars.as_str());
            capitalized
        }
        None => String::new(),
    }
}

/// Truncate a tool result to fit within the given height and width. If the text is valid JSON, we format it in a compact way before truncating.
/// This is a best-effort approach that may not work perfectly for text where 1 grapheme is rendered as multiple terminal cells.
pub(crate) fn format_and_truncate_tool_result(
    text: &str,
    max_lines: usize,
    line_width: usize,
) -> String {
    // Work out the maximum number of graphemes we can display for a result.
    // It's not guaranteed that 1 grapheme = 1 cell, so we subtract 1 per line as a fudge factor.
    // It also won't handle future terminal resizes properly, but it's an OK approximation for now.
    let max_graphemes = (max_lines * line_width).saturating_sub(max_lines);

    if let Some(formatted_json) = format_json_compact(text) {
        truncate_text(&formatted_json, max_graphemes)
    } else {
        truncate_text(text, max_graphemes)
    }
}

/// Format JSON text in a compact single-line format with spaces for better Ratatui wrapping.
/// Ex: `{"a":"b",c:["d","e"]}` -> `{"a": "b", "c": ["d", "e"]}`
/// Returns the formatted JSON string if the input is valid JSON, otherwise returns None.
/// This is a little complicated, but it's necessary because Ratatui's wrapping is *very* limited
/// and can only do line breaks at whitespace. If we use the default serde_json format, we get lines
/// without spaces that Ratatui can't wrap nicely. If we use the serde_json pretty format as-is,
/// it's much too sparse and uses too many terminal rows.
/// Relevant issue: https://github.com/ratatui/ratatui/issues/293
pub(crate) fn format_json_compact(text: &str) -> Option<String> {
    let json = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let json_pretty = serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string());

    // Convert multi-line pretty JSON to compact single-line format by removing newlines and excess whitespace
    let mut result = String::new();
    let mut chars = json_pretty.chars().peekable();
    let mut in_string = false;
    let mut escape_next = false;

    // Iterate over the characters in the JSON string, adding spaces after : and , but only when not in a string
    while let Some(ch) = chars.next() {
        match ch {
            '"' if !escape_next => {
                in_string = !in_string;
                result.push(ch);
            }
            '\\' if in_string => {
                escape_next = !escape_next;
                result.push(ch);
            }
            '\n' | '\r' if !in_string => {
                // Skip newlines when not in a string
            }
            ' ' | '\t' if !in_string => {
                // Add a space after : and , but only when not in a string
                if let Some(&next_ch) = chars.peek()
                    && let Some(last_ch) = result.chars().last()
                    && (last_ch == ':' || last_ch == ',')
                    && !matches!(next_ch, '}' | ']')
                {
                    result.push(' ');
                }
            }
            _ => {
                if escape_next && in_string {
                    escape_next = false;
                }
                result.push(ch);
            }
        }
    }

    Some(result)
}

/// Truncate `text` to `max_graphemes` graphemes. Using graphemes to avoid accidentally truncating in the middle of a multi-codepoint character.
pub(crate) fn truncate_text(text: &str, max_graphemes: usize) -> String {
    let mut graphemes = text.grapheme_indices(true);

    // Check if there's a grapheme at position max_graphemes (meaning there are more than max_graphemes total)
    if let Some((byte_index, _)) = graphemes.nth(max_graphemes) {
        // There are more than max_graphemes, so we need to truncate
        if max_graphemes >= 3 {
            // Truncate to max_graphemes - 3 and add "..." to stay within limit
            let mut truncate_graphemes = text.grapheme_indices(true);
            if let Some((truncate_byte_index, _)) = truncate_graphemes.nth(max_graphemes - 3) {
                let truncated = &text[..truncate_byte_index];
                format!("{truncated}...")
            } else {
                text.to_string()
            }
        } else {
            // max_graphemes < 3, so just return first max_graphemes without "..."
            let truncated = &text[..byte_index];
            truncated.to_string()
        }
    } else {
        // There are max_graphemes or fewer graphemes, return original text
        text.to_string()
    }
}

/// Truncate a path-like string to the given display width, keeping leading and trailing segments
/// where possible and inserting a single Unicode ellipsis between them. If an individual segment
/// cannot fit, it is front-truncated with an ellipsis.
pub(crate) fn center_truncate_path(path: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(path) <= max_width {
        return path.to_string();
    }

    let sep = std::path::MAIN_SEPARATOR;
    let has_leading_sep = path.starts_with(sep);
    let has_trailing_sep = path.ends_with(sep);
    let mut raw_segments: Vec<&str> = path.split(sep).collect();
    if has_leading_sep && !raw_segments.is_empty() && raw_segments[0].is_empty() {
        raw_segments.remove(0);
    }
    if has_trailing_sep
        && !raw_segments.is_empty()
        && raw_segments.last().is_some_and(|last| last.is_empty())
    {
        raw_segments.pop();
    }

    if raw_segments.is_empty() {
        if has_leading_sep {
            let root = sep.to_string();
            if UnicodeWidthStr::width(root.as_str()) <= max_width {
                return root;
            }
        }
        return "…".to_string();
    }

    struct Segment<'a> {
        original: &'a str,
        text: String,
        truncatable: bool,
        is_suffix: bool,
    }

    let assemble = |leading: bool, segments: &[Segment<'_>]| -> String {
        let mut result = String::new();
        if leading {
            result.push(sep);
        }
        for segment in segments {
            if !result.is_empty() && !result.ends_with(sep) {
                result.push(sep);
            }
            result.push_str(segment.text.as_str());
        }
        result
    };

    let front_truncate = |original: &str, allowed_width: usize| -> String {
        if allowed_width == 0 {
            return String::new();
        }
        if UnicodeWidthStr::width(original) <= allowed_width {
            return original.to_string();
        }
        if allowed_width == 1 {
            return "…".to_string();
        }

        let mut kept: Vec<char> = Vec::new();
        let mut used_width = 1; // reserve space for leading ellipsis
        for ch in original.chars().rev() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if used_width + ch_width > allowed_width {
                break;
            }
            used_width += ch_width;
            kept.push(ch);
        }
        kept.reverse();
        let mut truncated = String::from("…");
        for ch in kept {
            truncated.push(ch);
        }
        truncated
    };

    let mut combos: Vec<(usize, usize)> = Vec::new();
    let segment_count = raw_segments.len();
    for left in 1..=segment_count {
        let min_right = if left == segment_count { 0 } else { 1 };
        for right in min_right..=(segment_count - left) {
            combos.push((left, right));
        }
    }
    let desired_suffix = if segment_count > 1 {
        std::cmp::min(2, segment_count - 1)
    } else {
        0
    };
    let mut prioritized: Vec<(usize, usize)> = Vec::new();
    let mut fallback: Vec<(usize, usize)> = Vec::new();
    for combo in combos {
        if combo.1 >= desired_suffix {
            prioritized.push(combo);
        } else {
            fallback.push(combo);
        }
    }
    let sort_combos = |items: &mut Vec<(usize, usize)>| {
        items.sort_by(|(left_a, right_a), (left_b, right_b)| {
            left_b
                .cmp(left_a)
                .then_with(|| right_b.cmp(right_a))
                .then_with(|| (left_b + right_b).cmp(&(left_a + right_a)))
        });
    };
    sort_combos(&mut prioritized);
    sort_combos(&mut fallback);

    let fit_segments =
        |segments: &mut Vec<Segment<'_>>, allow_front_truncate: bool| -> Option<String> {
            loop {
                let candidate = assemble(has_leading_sep, segments);
                let width = UnicodeWidthStr::width(candidate.as_str());
                if width <= max_width {
                    return Some(candidate);
                }

                if !allow_front_truncate {
                    return None;
                }

                let mut indices: Vec<usize> = Vec::new();
                for (idx, seg) in segments.iter().enumerate().rev() {
                    if seg.truncatable && seg.is_suffix {
                        indices.push(idx);
                    }
                }
                for (idx, seg) in segments.iter().enumerate().rev() {
                    if seg.truncatable && !seg.is_suffix {
                        indices.push(idx);
                    }
                }

                if indices.is_empty() {
                    return None;
                }

                let mut changed = false;
                for idx in indices {
                    let original_width = UnicodeWidthStr::width(segments[idx].original);
                    if original_width <= max_width && segment_count > 2 {
                        continue;
                    }
                    let seg_width = UnicodeWidthStr::width(segments[idx].text.as_str());
                    let other_width = width.saturating_sub(seg_width);
                    let allowed_width = max_width.saturating_sub(other_width).max(1);
                    let new_text = front_truncate(segments[idx].original, allowed_width);
                    if new_text != segments[idx].text {
                        segments[idx].text = new_text;
                        changed = true;
                        break;
                    }
                }

                if !changed {
                    return None;
                }
            }
        };

    for (left_count, right_count) in prioritized.into_iter().chain(fallback.into_iter()) {
        let mut segments: Vec<Segment<'_>> = raw_segments[..left_count]
            .iter()
            .map(|seg| Segment {
                original: seg,
                text: (*seg).to_string(),
                truncatable: true,
                is_suffix: false,
            })
            .collect();

        let need_ellipsis = left_count + right_count < segment_count;
        if need_ellipsis {
            segments.push(Segment {
                original: "…",
                text: "…".to_string(),
                truncatable: false,
                is_suffix: false,
            });
        }

        if right_count > 0 {
            segments.extend(
                raw_segments[segment_count - right_count..]
                    .iter()
                    .map(|seg| Segment {
                        original: seg,
                        text: (*seg).to_string(),
                        truncatable: true,
                        is_suffix: true,
                    }),
            );
        }

        let allow_front_truncate = need_ellipsis || segment_count <= 2;
        if let Some(candidate) = fit_segments(&mut segments, allow_front_truncate) {
            return candidate;
        }
    }

    front_truncate(path, max_width)
}

/// Join a list of strings with proper English punctuation.
/// Examples:
/// - [] -> ""
/// - ["apple"] -> "apple"
/// - ["apple", "banana"] -> "apple and banana"
/// - ["apple", "banana", "cherry"] -> "apple, banana and cherry"
pub(crate) fn proper_join<T: AsRef<str>>(items: &[T]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].as_ref().to_string(),
        2 => format!("{} and {}", items[0].as_ref(), items[1].as_ref()),
        _ => {
            let last = items[items.len() - 1].as_ref();
            let mut result = String::new();

            for (i, item) in items.iter().take(items.len() - 1).enumerate() {
                if i > 0 {
                    result.push_str(", ");
                }
                result.push_str(item.as_ref());
            }

            format!("{result} and {last}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_truncate_text() {
        let text = "Hello, world!";
        let truncated = truncate_text(text, /*max_graphemes*/ 8);
        assert_eq!(truncated, "Hello...");
    }

    #[test]
    fn test_truncate_empty_string() {
        let text = "";
        let truncated = truncate_text(text, /*max_graphemes*/ 5);
        assert_eq!(truncated, "");
    }

    #[test]
    fn test_truncate_max_graphemes_zero() {
        let text = "Hello";
        let truncated = truncate_text(text, /*max_graphemes*/ 0);
        assert_eq!(truncated, "");
    }

    #[test]
    fn test_truncate_max_graphemes_one() {
        let text = "Hello";
        let truncated = truncate_text(text, /*max_graphemes*/ 1);
        assert_eq!(truncated, "H");
    }

    #[test]
    fn test_truncate_max_graphemes_two() {
        let text = "Hello";
        let truncated = truncate_text(text, /*max_graphemes*/ 2);
        assert_eq!(truncated, "He");
    }

    #[test]
    fn test_truncate_max_graphemes_three_boundary() {
        let text = "Hello";
        let truncated = truncate_text(text, /*max_graphemes*/ 3);
        assert_eq!(truncated, "...");
    }

    #[test]
    fn test_truncate_text_shorter_than_limit() {
        let text = "Hi";
        let truncated = truncate_text(text, /*max_graphemes*/ 10);
        assert_eq!(truncated, "Hi");
    }

    #[test]
    fn test_truncate_text_exact_length() {
        let text = "Hello";
        let truncated = truncate_text(text, /*max_graphemes*/ 5);
        assert_eq!(truncated, "Hello");
    }

    #[test]
    fn test_truncate_emoji() {
        let text = "👋🌍🚀✨💫";
        let truncated = truncate_text(text, /*max_graphemes*/ 3);
        assert_eq!(truncated, "...");

        let truncated_longer = truncate_text(text, /*max_graphemes*/ 4);
        assert_eq!(truncated_longer, "👋...");
    }

    #[test]
    fn test_truncate_unicode_combining_characters() {
        let text = "é́ñ̃"; // Characters with combining marks
        let truncated = truncate_text(text, /*max_graphemes*/ 2);
        assert_eq!(truncated, "é́ñ̃");
    }

    #[test]
    fn test_truncate_very_long_text() {
        let text = "a".repeat(1000);
        let truncated = truncate_text(&text, /*max_graphemes*/ 10);
        assert_eq!(truncated, "aaaaaaa...");
        assert_eq!(truncated.len(), 10); // 7 'a's + 3 dots
    }

    #[test]
    fn test_format_json_compact_simple_object() {
        let json = r#"{ "name": "John", "age": 30 }"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(result, r#"{"name": "John", "age": 30}"#);
    }

    #[test]
    fn test_format_json_compact_nested_object() {
        let json = r#"{ "user": { "name": "John", "details": { "age": 30, "city": "NYC" } } }"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(
            result,
            r#"{"user": {"name": "John", "details": {"age": 30, "city": "NYC"}}}"#
        );
    }

    #[test]
    fn test_center_truncate_doesnt_truncate_short_path() {
        let sep = std::path::MAIN_SEPARATOR;
        let path = format!("{sep}Users{sep}codex{sep}Public");
        let truncated = center_truncate_path(&path, /*max_width*/ 40);

        assert_eq!(truncated, path);
    }

    #[test]
    fn test_center_truncate_truncates_long_path() {
        let sep = std::path::MAIN_SEPARATOR;
        let path = format!("~{sep}hello{sep}the{sep}fox{sep}is{sep}very{sep}fast");
        let truncated = center_truncate_path(&path, /*max_width*/ 24);

        assert_eq!(
            truncated,
            format!("~{sep}hello{sep}the{sep}…{sep}very{sep}fast")
        );
    }

    #[test]
    fn test_center_truncate_truncates_long_windows_path() {
        let sep = std::path::MAIN_SEPARATOR;
        let path = format!(
            "C:{sep}Users{sep}codex{sep}Projects{sep}super{sep}long{sep}windows{sep}path{sep}file.txt"
        );
        let truncated = center_truncate_path(&path, /*max_width*/ 36);

        let expected = format!("C:{sep}Users{sep}codex{sep}…{sep}path{sep}file.txt");

        assert_eq!(truncated, expected);
    }

    #[test]
    fn test_center_truncate_handles_long_segment() {
        let sep = std::path::MAIN_SEPARATOR;
        let path = format!("~{sep}supercalifragilisticexpialidocious");
        let truncated = center_truncate_path(&path, /*max_width*/ 18);

        assert_eq!(truncated, format!("~{sep}…cexpialidocious"));
    }

    #[test]
    fn test_format_json_compact_array() {
        let json = r#"[ 1, 2, { "key": "value" }, "string" ]"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(result, r#"[1, 2, {"key": "value"}, "string"]"#);
    }

    #[test]
    fn test_format_json_compact_already_compact() {
        let json = r#"{"compact":true}"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(result, r#"{"compact": true}"#);
    }

    #[test]
    fn test_format_json_compact_with_whitespace() {
        let json = r#"
        {
            "name": "John",
            "hobbies": [
                "reading",
                "coding"
            ]
        }
        "#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(
            result,
            r#"{"name": "John", "hobbies": ["reading", "coding"]}"#
        );
    }

    #[test]
    fn test_format_json_compact_invalid_json() {
        let invalid_json = r#"{"invalid": json syntax}"#;
        let result = format_json_compact(invalid_json);
        assert!(result.is_none());
    }

    #[test]
    fn test_format_json_compact_empty_object() {
        let json = r#"{}"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(result, "{}");
    }

    #[test]
    fn test_format_json_compact_empty_array() {
        let json = r#"[]"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(result, "[]");
    }

    #[test]
    fn test_format_json_compact_primitive_values() {
        assert_eq!(format_json_compact("42").unwrap(), "42");
        assert_eq!(format_json_compact("true").unwrap(), "true");
        assert_eq!(format_json_compact("false").unwrap(), "false");
        assert_eq!(format_json_compact("null").unwrap(), "null");
        assert_eq!(format_json_compact(r#""string""#).unwrap(), r#""string""#);
    }

    #[test]
    fn test_proper_join() {
        let empty: Vec<String> = vec![];
        assert_eq!(proper_join(&empty), "");
        assert_eq!(proper_join(&["apple"]), "apple");
        assert_eq!(proper_join(&["apple", "banana"]), "apple and banana");
        assert_eq!(
            proper_join(&["apple", "banana", "cherry"]),
            "apple, banana and cherry"
        );
        assert_eq!(
            proper_join(&["apple", "banana", "cherry", "date"]),
            "apple, banana, cherry and date"
        );
    }
}
pub(crate) fn strip_eq_tags(input: &str) -> String {
    if !input.contains("<eq ") {
        return input.to_string();
    }

    let mut result = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = remaining.find("<eq ") {
        result.push_str(&remaining[..start]);

        let after_eq = &remaining[start..];

        // Extract the latex attribute value.
        let latex = extract_latex_attr(after_eq).unwrap_or_default();

        // Determine if this is a display (block) equation.
        let gt_pos = after_eq.find('>');
        let is_display = if let Some(gt) = gt_pos {
            after_eq[..gt].contains("display=\"block\"")
        } else {
            false
        };

        // Check if self-closing: the `/>` must appear before the first `>`.
        let self_close_pos = after_eq.find("/>");
        let is_self_closing = match (self_close_pos, gt_pos) {
            (Some(sc), Some(gt)) => sc < gt || sc == gt.saturating_sub(1),
            (Some(_), None) => true,
            _ => false,
        };

        if is_self_closing {
            if is_display {
                result.push_str("$$");
                result.push_str(&latex);
                result.push_str("$$");
            } else {
                result.push('$');
                result.push_str(&latex);
                result.push('$');
            }
            if let Some(pos) = self_close_pos {
                remaining = &after_eq[pos + 2..];
            }
        } else if let Some(close_tag) = after_eq.find("</eq>") {
            // Non-self-closing: skip everything between <eq ...> and </eq>.
            if is_display {
                result.push_str("$$");
                result.push_str(&latex);
                result.push_str("$$");
            } else {
                result.push('$');
                result.push_str(&latex);
                result.push('$');
            }
            remaining = &after_eq[close_tag + 5..];
        } else if let Some(gt) = gt_pos {
            // Malformed: no closing tag.  Skip the opening tag.
            remaining = &after_eq[gt + 1..];
        } else {
            // No `>` at all — append as-is and stop.
            result.push_str(after_eq);
            remaining = "";
        }
    }

    result.push_str(remaining);
    result
}

/// Extract the value of the `latex="..."` attribute from an `<eq ...>` tag
/// fragment.
fn extract_latex_attr(tag: &str) -> Option<String> {
    let marker = "latex=\"";
    let latex_start = tag.find(marker)? + marker.len();
    let rest = &tag[latex_start..];
    let latex_end = rest.find('"')?;
    Some(rest[..latex_end].to_string())
}
pub(crate) fn latex_to_plain_text(input: &str) -> String {
    if !input.contains('$') {
        return input.to_string();
    }

    let mut result = String::with_capacity(input.len());
    let mut remaining = input;

    while !remaining.is_empty() {
        // Look for display math ($$...$$) first, then inline math ($...$).
        if let Some(start) = remaining.find('$') {
            result.push_str(&remaining[..start]);
            let after_dollar = &remaining[start..];

            if after_dollar.starts_with("$$") {
                // Display math: $$...$$
                let inner_start = 2;
                if let Some(end) = after_dollar[inner_start..].find("$$") {
                    let latex = &after_dollar[inner_start..inner_start + end];
                    result.push_str(&convert_latex_expr(latex));
                    remaining = &after_dollar[inner_start + end + 2..];
                } else {
                    // No closing $$ — emit as-is and stop.
                    result.push_str(after_dollar);
                    remaining = "";
                }
            } else {
                // Inline math: $...$
                let inner_start = 1;
                if let Some(end) = after_dollar[inner_start..].find('$') {
                    let latex = &after_dollar[inner_start..inner_start + end];
                    // Guard against false positives: if the "latex" contains
                    // a newline it's probably not math (e.g. shell $VAR on
                    // different lines).
                    if latex.contains('\n') || latex.is_empty() {
                        result.push('$');
                        remaining = &after_dollar[1..];
                    } else {
                        result.push_str(&convert_latex_expr(latex));
                        remaining = &after_dollar[inner_start + end + 1..];
                    }
                } else {
                    // No closing $ — emit as-is and stop.
                    result.push_str(after_dollar);
                    remaining = "";
                }
            }
        } else {
            result.push_str(remaining);
            remaining = "";
        }
    }

    result
}

/// Convert a single LaTeX expression (without delimiters) to plain text.
fn convert_latex_expr(latex: &str) -> String {
    let mut s = latex.to_string();

    // ── Structural commands ─────────────────────────────────────────────
    // \frac{a}{b} → a/b
    while let Some(pos) = s.find("\\frac") {
        let after = &s[pos + 5..];
        if let Some((num, den, total_len)) = extract_two_brace_groups(after) {
            let replacement = format!("{num}/{den}");
            s = format!("{}{replacement}{}", &s[..pos], &s[pos + 5 + total_len..]);
        } else {
            break;
        }
    }

    // \sqrt{x} → sqrt(x),  \sqrt[n]{x} → x^(1/n)
    while let Some(pos) = s.find("\\sqrt") {
        let after = &s[pos + 5..];
        if after.starts_with('[') {
            // \sqrt[n]{x}
            if let Some(bracket_end) = after.find(']') {
                let n = &after[1..bracket_end];
                let brace_after = &after[bracket_end + 1..];
                if let Some((content, brace_len)) = extract_brace_group(brace_after) {
                    let replacement = format!("{content}^(1/{n})");
                    let total = 5 + bracket_end + 1 + brace_len;
                    s = format!("{}{replacement}{}", &s[..pos], &s[pos + total..]);
                } else {
                    break;
                }
            } else {
                break;
            }
        } else if let Some((content, brace_len)) = extract_brace_group(after) {
            let replacement = format!("sqrt({content})");
            s = format!("{}{replacement}{}", &s[..pos], &s[pos + 5 + brace_len..]);
        } else {
            break;
        }
    }

    // \text{...} and \mathrm{...} → content
    for cmd in &[
        "\\text",
        "\\mathrm",
        "\\textbf",
        "\\textit",
        "\\mathbf",
        "\\mathit",
        "\\mathcal",
        "\\mathbb",
        "\\operatorname",
    ] {
        while let Some(pos) = s.find(cmd) {
            let after = &s[pos + cmd.len()..];
            if let Some((content, brace_len)) = extract_brace_group(after) {
                s = format!(
                    "{}{content}{}",
                    &s[..pos],
                    &s[pos + cmd.len() + brace_len..]
                );
            } else {
                break;
            }
        }
    }

    // \left and \right delimiters — just remove the command prefix.
    s = s.replace("\\left(", "(");
    s = s.replace("\\right)", ")");
    s = s.replace("\\left[", "[");
    s = s.replace("\\right]", "]");
    s = s.replace("\\left\\{", "{");
    s = s.replace("\\right\\}", "}");
    s = s.replace("\\left|", "|");
    s = s.replace("\\right|", "|");
    s = s.replace("\\left.", "");
    s = s.replace("\\right.", "");

    // ── Greek letters ───────────────────────────────────────────────────
    let greek: &[(&str, &str)] = &[
        ("\\alpha", "\u{03B1}"),
        ("\\beta", "\u{03B2}"),
        ("\\gamma", "\u{03B3}"),
        ("\\delta", "\u{03B4}"),
        ("\\epsilon", "\u{03B5}"),
        ("\\varepsilon", "\u{03B5}"),
        ("\\zeta", "\u{03B6}"),
        ("\\eta", "\u{03B7}"),
        ("\\theta", "\u{03B8}"),
        ("\\vartheta", "\u{03D1}"),
        ("\\iota", "\u{03B9}"),
        ("\\kappa", "\u{03BA}"),
        ("\\lambda", "\u{03BB}"),
        ("\\mu", "\u{03BC}"),
        ("\\nu", "\u{03BD}"),
        ("\\xi", "\u{03BE}"),
        ("\\pi", "\u{03C0}"),
        ("\\rho", "\u{03C1}"),
        ("\\sigma", "\u{03C3}"),
        ("\\tau", "\u{03C4}"),
        ("\\upsilon", "\u{03C5}"),
        ("\\phi", "\u{03C6}"),
        ("\\varphi", "\u{03C6}"),
        ("\\chi", "\u{03C7}"),
        ("\\psi", "\u{03C8}"),
        ("\\omega", "\u{03C9}"),
        ("\\Gamma", "\u{0393}"),
        ("\\Delta", "\u{0394}"),
        ("\\Theta", "\u{0398}"),
        ("\\Lambda", "\u{039B}"),
        ("\\Xi", "\u{039E}"),
        ("\\Pi", "\u{03A0}"),
        ("\\Sigma", "\u{03A3}"),
        ("\\Phi", "\u{03A6}"),
        ("\\Psi", "\u{03A8}"),
        ("\\Omega", "\u{03A9}"),
    ];

    // Replace Greek letters — must check word boundaries to avoid partial
    // matches (e.g. \epsilon before \varepsilon is handled by ordering, but
    // we also need to avoid replacing inside longer commands).
    for &(cmd, unicode) in greek {
        // Replace command when followed by a non-alpha character or end-of-string.
        let mut out = String::with_capacity(s.len());
        let mut rest = s.as_str();
        while let Some(pos) = rest.find(cmd) {
            out.push_str(&rest[..pos]);
            let after_cmd = &rest[pos + cmd.len()..];
            // Check that the character after the command is not alphabetic
            // (to avoid matching \pi inside \psi, etc.)
            if after_cmd
                .chars()
                .next()
                .is_none_or(|c| !c.is_ascii_alphabetic())
            {
                out.push_str(unicode);
                rest = after_cmd;
            } else {
                out.push_str(cmd);
                rest = after_cmd;
            }
        }
        out.push_str(rest);
        s = out;
    }

    // ── Operators and symbols ───────────────────────────────────────────
    let symbols: &[(&str, &str)] = &[
        ("\\times", "\u{00D7}"),
        ("\\cdot", "\u{00B7}"),
        ("\\div", "\u{00F7}"),
        ("\\pm", "\u{00B1}"),
        ("\\mp", "\u{2213}"),
        ("\\leq", "\u{2264}"),
        ("\\geq", "\u{2265}"),
        ("\\neq", "\u{2260}"),
        ("\\approx", "\u{2248}"),
        ("\\equiv", "\u{2261}"),
        ("\\sim", "\u{223C}"),
        ("\\propto", "\u{221D}"),
        ("\\infty", "\u{221E}"),
        ("\\partial", "\u{2202}"),
        ("\\nabla", "\u{2207}"),
        ("\\sum", "\u{2211}"),
        ("\\prod", "\u{220F}"),
        ("\\int", "\u{222B}"),
        ("\\iint", "\u{222C}"),
        ("\\iiint", "\u{222D}"),
        ("\\oint", "\u{222E}"),
        ("\\forall", "\u{2200}"),
        ("\\exists", "\u{2203}"),
        ("\\in", "\u{2208}"),
        ("\\notin", "\u{2209}"),
        ("\\subset", "\u{2282}"),
        ("\\supset", "\u{2283}"),
        ("\\subseteq", "\u{2286}"),
        ("\\supseteq", "\u{2287}"),
        ("\\cup", "\u{222A}"),
        ("\\cap", "\u{2229}"),
        ("\\emptyset", "\u{2205}"),
        ("\\varnothing", "\u{2205}"),
        ("\\neg", "\u{00AC}"),
        ("\\land", "\u{2227}"),
        ("\\lor", "\u{2228}"),
        ("\\rightarrow", "\u{2192}"),
        ("\\leftarrow", "\u{2190}"),
        ("\\Rightarrow", "\u{21D2}"),
        ("\\Leftarrow", "\u{21D0}"),
        ("\\leftrightarrow", "\u{2194}"),
        ("\\Leftrightarrow", "\u{21D4}"),
        ("\\mapsto", "\u{21A6}"),
        ("\\to", "\u{2192}"),
        ("\\gets", "\u{2190}"),
        ("\\uparrow", "\u{2191}"),
        ("\\downarrow", "\u{2193}"),
        ("\\ldots", "\u{2026}"),
        ("\\cdots", "\u{22EF}"),
        ("\\vdots", "\u{22EE}"),
        ("\\ddots", "\u{22F1}"),
        ("\\langle", "\u{27E8}"),
        ("\\rangle", "\u{27E9}"),
        ("\\lceil", "\u{2308}"),
        ("\\rceil", "\u{2309}"),
        ("\\lfloor", "\u{230A}"),
        ("\\rfloor", "\u{230B}"),
        ("\\circ", "\u{2218}"),
        ("\\star", "\u{22C6}"),
        ("\\bullet", "\u{2022}"),
        ("\\oplus", "\u{2295}"),
        ("\\otimes", "\u{2297}"),
        ("\\dagger", "\u{2020}"),
        ("\\ell", "\u{2113}"),
        ("\\hbar", "\u{210F}"),
        ("\\Re", "\u{211C}"),
        ("\\Im", "\u{2111}"),
        ("\\aleph", "\u{2135}"),
        ("\\wp", "\u{2118}"),
    ];

    for &(cmd, unicode) in symbols {
        s = s.replace(cmd, unicode);
    }

    // ── Subscripts and superscripts ─────────────────────────────────────
    // Convert ^{...} to Unicode superscripts where possible, else (...)
    // Convert _{...} to Unicode subscripts where possible, else (...)
    s = convert_scripts(&s);

    // ── Cleanup ─────────────────────────────────────────────────────────
    // Remove remaining braces used for grouping.
    s = s.replace('{', "");
    s = s.replace('}', "");
    // Remove \, \; \! \: \> (spacing commands).
    s = s.replace("\\,", "");
    s = s.replace("\\;", " ");
    s = s.replace("\\!", "");
    s = s.replace("\\:", " ");
    s = s.replace("\\>", " ");
    s = s.replace("\\quad", " ");
    s = s.replace("\\qquad", "  ");
    // Remove \displaystyle and \textstyle.
    s = s.replace("\\displaystyle", "");
    s = s.replace("\\textstyle", "");
    // Mid-bar.
    s = s.replace("\\mid", "|");
    s = s.replace("\\|", "||");
    // Clean up any remaining backslash commands we don't recognize — strip
    // the backslash and keep the command name as readable text.
    s = strip_unknown_commands(&s);
    // Collapse multiple spaces.
    while s.contains("  ") {
        s = s.replace("  ", " ");
    }

    s.trim().to_string()
}

/// Extract content inside `{...}` at the start of `s`.
/// Returns `(content, bytes_consumed)` where bytes_consumed includes the braces.
fn extract_brace_group(s: &str) -> Option<(String, usize)> {
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((s[1..i].to_string(), i + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// Extract two consecutive brace groups: `{a}{b}`.
/// Returns `(a_content, b_content, total_bytes_consumed)`.
fn extract_two_brace_groups(s: &str) -> Option<(String, String, usize)> {
    let (first, len1) = extract_brace_group(s)?;
    let (second, len2) = extract_brace_group(&s[len1..])?;
    Some((first, second, len1 + len2))
}

/// Convert `^` and `_` scripts to Unicode equivalents where possible.
fn convert_scripts(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if (ch == '^' || ch == '_') && chars.peek().is_some() {
            let is_super = ch == '^';
            // Collect the script content.
            let content = if chars.peek() == Some(&'{') {
                // Brace group: ^{abc} or _{abc}
                chars.next(); // consume '{'
                let mut depth = 1;
                let mut buf = String::new();
                for c in chars.by_ref() {
                    match c {
                        '{' => {
                            depth += 1;
                            buf.push(c);
                        }
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            buf.push(c);
                        }
                        _ => buf.push(c),
                    }
                }
                buf
            } else {
                // Single character: ^2 or _i
                chars.next().map(|c| c.to_string()).unwrap_or_default()
            };

            if is_super {
                if let Some(uni) = to_unicode_superscript(&content) {
                    result.push_str(&uni);
                } else {
                    result.push('^');
                    result.push('(');
                    result.push_str(&content);
                    result.push(')');
                }
            } else if let Some(uni) = to_unicode_subscript(&content) {
                result.push_str(&uni);
            } else {
                result.push('_');
                result.push('(');
                result.push_str(&content);
                result.push(')');
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Try to convert a string to Unicode superscript characters.
/// Returns `None` if any character can't be converted.
fn to_unicode_superscript(s: &str) -> Option<String> {
    let mut result = String::with_capacity(s.len() * 3);
    for ch in s.chars() {
        let sup = match ch {
            '0' => '\u{2070}',
            '1' => '\u{00B9}',
            '2' => '\u{00B2}',
            '3' => '\u{00B3}',
            '4' => '\u{2074}',
            '5' => '\u{2075}',
            '6' => '\u{2076}',
            '7' => '\u{2077}',
            '8' => '\u{2078}',
            '9' => '\u{2079}',
            '+' => '\u{207A}',
            '-' => '\u{207B}',
            '=' => '\u{207C}',
            '(' => '\u{207D}',
            ')' => '\u{207E}',
            'n' => '\u{207F}',
            'i' => '\u{2071}',
            '*' => '\u{204E}',
            'T' => '\u{1D40}',
            _ => return None,
        };
        result.push(sup);
    }
    Some(result)
}

/// Try to convert a string to Unicode subscript characters.
/// Returns `None` if any character can't be converted.
fn to_unicode_subscript(s: &str) -> Option<String> {
    let mut result = String::with_capacity(s.len() * 3);
    for ch in s.chars() {
        let sub = match ch {
            '0' => '\u{2080}',
            '1' => '\u{2081}',
            '2' => '\u{2082}',
            '3' => '\u{2083}',
            '4' => '\u{2084}',
            '5' => '\u{2085}',
            '6' => '\u{2086}',
            '7' => '\u{2087}',
            '8' => '\u{2088}',
            '9' => '\u{2089}',
            '+' => '\u{208A}',
            '-' => '\u{208B}',
            '=' => '\u{208C}',
            '(' => '\u{208D}',
            ')' => '\u{208E}',
            'a' => '\u{2090}',
            'e' => '\u{2091}',
            'h' => '\u{2095}',
            'i' => '\u{1D62}',
            'j' => '\u{2C7C}',
            'k' => '\u{2096}',
            'l' => '\u{2097}',
            'm' => '\u{2098}',
            'n' => '\u{2099}',
            'o' => '\u{2092}',
            'p' => '\u{209A}',
            'r' => '\u{1D63}',
            's' => '\u{209B}',
            't' => '\u{209C}',
            'u' => '\u{1D64}',
            'v' => '\u{1D65}',
            'x' => '\u{2093}',
            _ => return None,
        };
        result.push(sub);
    }
    Some(result)
}

/// Strip unrecognised `\command` sequences, keeping the command name as
/// readable text (e.g. `\mathcal` → `mathcal`).
fn strip_unknown_commands(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch == '\\' {
            // Peek: if next char is alphabetic, consume the command name.
            let mut cmd = String::new();
            while let Some(&(_, next)) = chars.peek() {
                if next.is_ascii_alphabetic() {
                    cmd.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            if cmd.is_empty() {
                // Escaped non-alpha character (\\ , \& etc.) — keep the char.
                if let Some(&(_, next)) = chars.peek() {
                    result.push(next);
                    chars.next();
                }
            }
            // else: drop the \command, the name is not useful as text
        } else {
            result.push(ch);
        }
    }
    result
}

/// Strip `<voice>` / `</voice>` wrapper tags (with or without attributes,
/// case-insensitive) from text, then strip `<eq>` tags via [`strip_eq_tags`].
///
/// Handles bare `<voice>`, `<Voice>`, `<voice name="alloy">`, and any other
/// attribute variations.  This is the single canonical implementation used by
/// all display paths.
pub(crate) fn strip_voice_tags(text: &str) -> String {
    if !text.contains('<') {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find('<') {
        result.push_str(&remaining[..start]);
        let tag_region = &remaining[start..];
        let tag_lower = tag_region.to_ascii_lowercase();

        if tag_lower.starts_with("<voice") {
            if let Some(end) = tag_region.find('>') {
                remaining = &tag_region[end + 1..];
                continue;
            }
            remaining = &tag_region["<voice".len()..];
            continue;
        }

        if tag_lower.starts_with("</voice>") {
            remaining = &tag_region["</voice>".len()..];
            continue;
        }

        if tag_lower.starts_with("</voice") {
            let suffix = &tag_region["</voice".len()..];
            let malformed_close = suffix.is_empty()
                || suffix.chars().next().is_some_and(|ch| {
                    ch.is_ascii_alphanumeric() || matches!(ch, '\'' | '"' | '_' | '-')
                });
            if malformed_close {
                remaining = suffix;
                continue;
            }
        }

        result.push('<');
        remaining = &tag_region[1..];
    }
    result.push_str(remaining);
    strip_eq_tags(&result)
}

#[cfg(test)]
mod ata_reading_view_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_strip_eq_tags_no_tags() {
        assert_eq!(strip_eq_tags("hello world"), "hello world");
    }

    #[test]
    fn test_strip_eq_tags_inline() {
        let input = r#"Consider <eq latex="x^2 + 1">x squared plus one</eq> here."#;
        assert_eq!(strip_eq_tags(input), "Consider $x^2 + 1$ here.");
    }

    #[test]
    fn test_strip_eq_tags_display_block() {
        let input =
            r#"The formula is <eq latex="E=mc^2" display="block">E equals m c squared</eq> above."#;
        assert_eq!(strip_eq_tags(input), "The formula is $$E=mc^2$$ above.");
    }

    #[test]
    fn test_strip_eq_tags_self_closing() {
        let input = r#"See <eq latex="a+b" speak="a plus b"/> next."#;
        assert_eq!(strip_eq_tags(input), "See $a+b$ next.");
    }

    #[test]
    fn test_strip_eq_tags_multiple() {
        let input = r#"<eq latex="x">ex</eq> and <eq latex="y">why</eq>"#;
        assert_eq!(strip_eq_tags(input), "$x$ and $y$");
    }

    #[test]
    fn test_proper_join() {
        let empty: Vec<String> = vec![];
        assert_eq!(proper_join(&empty), "");
        assert_eq!(proper_join(&["apple"]), "apple");
        assert_eq!(proper_join(&["apple", "banana"]), "apple and banana");
        assert_eq!(
            proper_join(&["apple", "banana", "cherry"]),
            "apple, banana and cherry"
        );
        assert_eq!(
            proper_join(&["apple", "banana", "cherry", "date"]),
            "apple, banana, cherry and date"
        );
    }

    #[test]
    fn test_strip_voice_tags_bare() {
        assert_eq!(strip_voice_tags("<voice>hello</voice>"), "hello");
    }

    #[test]
    fn test_strip_voice_tags_with_name_attribute() {
        assert_eq!(
            strip_voice_tags("<voice name=\"alloy\">hello</voice>"),
            "hello"
        );
    }

    #[test]
    fn test_strip_voice_tags_case_insensitive() {
        assert_eq!(strip_voice_tags("<Voice>hello</Voice>"), "hello");
    }

    #[test]
    fn test_strip_voice_tags_multiple_attributes() {
        assert_eq!(
            strip_voice_tags("<voice name=\"shimmer\" style=\"fast\">text</voice>"),
            "text"
        );
    }

    #[test]
    fn test_strip_voice_tags_no_tags() {
        assert_eq!(strip_voice_tags("no tags here"), "no tags here");
    }

    #[test]
    fn test_strip_voice_tags_preserves_other_tags() {
        assert_eq!(
            strip_voice_tags("<b>bold</b> <voice>hello</voice>"),
            "<b>bold</b> hello"
        );
    }

    #[test]
    fn test_strip_voice_tags_with_eq_tags() {
        // The function should also strip eq tags (via strip_eq_tags).
        assert_eq!(
            strip_voice_tags("<voice name=\"alloy\"><eq latex=\"x^2\">x squared</eq></voice>"),
            "$x^2$"
        );
    }

    #[test]
    fn test_strip_voice_tags_no_angle_brackets() {
        // Fast-path: no '<' in input.
        assert_eq!(strip_voice_tags("plain text"), "plain text");
    }

    // ─── Comprehensive voice tag stripping tests ────────────────────────

    #[test]
    fn test_strip_voice_tags_with_name_alloy() {
        assert_eq!(
            strip_voice_tags(r#"<voice name="alloy">hello</voice>"#),
            "hello"
        );
    }

    #[test]
    fn test_strip_voice_tags_with_multiple_attributes() {
        assert_eq!(
            strip_voice_tags(r#"<voice name="shimmer" style="fast">text</voice>"#),
            "text"
        );
    }

    #[test]
    fn test_strip_voice_tags_case_insensitive_mixed() {
        assert_eq!(
            strip_voice_tags(r#"<Voice NAME="alloy">text</Voice>"#),
            "text"
        );
    }

    #[test]
    fn test_strip_voice_tags_nested_with_eq() {
        // Voice tag wrapping an eq tag — both should be stripped, eq replaced with LaTeX.
        assert_eq!(
            strip_voice_tags(
                r#"<voice name="alloy">before <eq latex="x^2">x squared</eq> after</voice>"#
            ),
            "before $x^2$ after"
        );
    }

    #[test]
    fn test_strip_voice_tags_multiple_voice_regions() {
        assert_eq!(
            strip_voice_tags(
                r#"<voice name="alloy">first</voice> middle <voice name="shimmer">second</voice>"#
            ),
            "first middle second"
        );
    }

    #[test]
    fn test_strip_voice_tags_empty_voice() {
        assert_eq!(strip_voice_tags(r#"<voice name="alloy"></voice>"#), "");
    }

    #[test]
    fn test_strip_voice_tags_unclosed_opening_tag() {
        // Opening tag without closing — the opening tag itself is stripped,
        // and remaining text passes through.
        let result = strip_voice_tags(r#"<voice name="alloy">text"#);
        assert_eq!(result, "text");
    }

    #[test]
    fn test_strip_voice_tags_preserves_non_voice_html() {
        assert_eq!(strip_voice_tags("<p>hello</p>"), "<p>hello</p>");
    }

    #[test]
    fn test_strip_voice_tags_self_closing() {
        // Self-closing <voice/> should be stripped.
        assert_eq!(strip_voice_tags("<voice/>"), "");
    }

    #[test]
    fn test_strip_voice_tags_with_eq_tags_mixed() {
        // Voice wrapping eq without name attribute.
        assert_eq!(
            strip_voice_tags(r#"<voice><eq latex="E=mc^2">E equals mc squared</eq></voice>"#),
            "$E=mc^2$"
        );
    }

    #[test]
    fn test_strip_voice_tags_preserves_surrounding_text() {
        assert_eq!(
            strip_voice_tags(r#"before <voice name="alloy">inner</voice> after"#),
            "before inner after"
        );
    }

    #[test]
    fn test_strip_voice_tags_consecutive_voice_tags() {
        assert_eq!(
            strip_voice_tags(r#"<voice name="a">one</voice><voice name="b">two</voice>"#),
            "onetwo"
        );
    }

    #[test]
    fn test_strip_voice_tags_voice_with_newlines() {
        assert_eq!(
            strip_voice_tags("<voice name=\"alloy\">line1\nline2</voice>"),
            "line1\nline2"
        );
    }

    #[test]
    fn test_strip_voice_tags_only_angle_brackets_no_voice() {
        // Has '<' but not a voice tag — should not be stripped.
        assert_eq!(strip_voice_tags("5 < 10 > 3"), "5 < 10 > 3");
    }

    #[test]
    fn test_strip_voice_tags_empty_input() {
        assert_eq!(strip_voice_tags(""), "");
    }

    #[test]
    fn test_strip_voice_tags_bare_voice_with_eq_inside() {
        // Bare <voice> wrapping eq tags — both stripped.
        assert_eq!(
            strip_voice_tags(r#"<voice>See <eq latex="\pi">pi</eq> here</voice>"#),
            r"See $\pi$ here"
        );
    }

    // ─── LaTeX to plain text tests ──────────────────────────────────────

    #[test]
    fn test_latex_no_math() {
        assert_eq!(latex_to_plain_text("hello world"), "hello world");
    }

    #[test]
    fn test_latex_simple_pi() {
        assert_eq!(latex_to_plain_text(r"$\pi$"), "\u{03C0}");
    }

    #[test]
    fn test_latex_inline_greek() {
        assert_eq!(
            latex_to_plain_text(r"the value $\alpha + \beta$ is important"),
            "the value \u{03B1} + \u{03B2} is important"
        );
    }

    #[test]
    fn test_latex_display_math() {
        assert_eq!(latex_to_plain_text(r"$$E = mc^2$$"), "E = mc\u{00B2}");
    }

    #[test]
    fn test_latex_superscript_simple() {
        assert_eq!(latex_to_plain_text(r"$x^2$"), "x\u{00B2}");
    }

    #[test]
    fn test_latex_subscript_simple() {
        assert_eq!(latex_to_plain_text(r"$x_0$"), "x\u{2080}");
    }

    #[test]
    fn test_latex_frac() {
        assert_eq!(latex_to_plain_text(r"$\frac{a}{b}$"), "a/b");
    }

    #[test]
    fn test_latex_sqrt() {
        assert_eq!(latex_to_plain_text(r"$\sqrt{x}$"), "sqrt(x)");
    }

    #[test]
    fn test_latex_complex_policy_gradient() {
        // The motivating example from the bug report.
        let input = r"$\pi_\theta(a_t \mid o_t)$";
        let result = latex_to_plain_text(input);
        // Should produce readable text without raw LaTeX backslashes.
        assert!(
            !result.contains('\\'),
            "result should not contain backslashes: {result}"
        );
        assert!(
            !result.contains('$'),
            "result should not contain dollar signs: {result}"
        );
        // Should contain pi symbol.
        assert!(
            result.contains('\u{03C0}'),
            "result should contain pi: {result}"
        );
    }

    #[test]
    fn test_latex_operators() {
        assert_eq!(latex_to_plain_text(r"$a \times b$"), "a \u{00D7} b");
        assert_eq!(latex_to_plain_text(r"$a \leq b$"), "a \u{2264} b");
    }

    #[test]
    fn test_latex_sum() {
        let result = latex_to_plain_text(r"$\sum_{i=1}^{n} x_i$");
        assert!(
            result.contains('\u{2211}'),
            "should contain sum symbol: {result}"
        );
        assert!(!result.contains('\\'), "no backslashes: {result}");
    }

    #[test]
    fn test_latex_integral() {
        let result = latex_to_plain_text(r"$\int_0^1 f(x) dx$");
        assert!(
            result.contains('\u{222B}'),
            "should contain integral symbol: {result}"
        );
    }

    #[test]
    fn test_latex_text_command() {
        assert_eq!(latex_to_plain_text(r"$\text{hello}$"), "hello");
    }

    #[test]
    fn test_latex_mixed_text_and_math() {
        let input = "Consider $\\pi$ and then $x^2 + y^2 = r^2$ in context.";
        let result = latex_to_plain_text(input);
        assert!(result.contains('\u{03C0}'), "should contain pi: {result}");
        assert!(
            result.contains('\u{00B2}'),
            "should contain superscript 2: {result}"
        );
        assert!(!result.contains('$'), "no dollar signs: {result}");
    }

    #[test]
    fn test_latex_preserves_non_math_dollars() {
        // A lone $ without a closing match should be preserved.
        assert_eq!(latex_to_plain_text("costs $5"), "costs $5");
    }

    #[test]
    fn test_latex_empty_math() {
        // Empty $$ should be preserved (not treated as math).
        assert_eq!(latex_to_plain_text("a $$ b"), "a $$ b");
    }

    #[test]
    fn test_latex_left_right_parens() {
        let result = latex_to_plain_text(r"$\left( a + b \right)$");
        assert_eq!(result, "( a + b )");
    }

    #[test]
    fn test_latex_nabla() {
        assert_eq!(latex_to_plain_text(r"$\nabla f$"), "\u{2207} f");
    }

    #[test]
    fn test_latex_infty() {
        assert_eq!(latex_to_plain_text(r"$\infty$"), "\u{221E}");
    }

    #[test]
    fn test_latex_greek_word_boundary() {
        // \pi should not match inside \psi.
        let result = latex_to_plain_text(r"$\psi$");
        assert_eq!(result, "\u{03C8}");
    }

    #[test]
    fn test_latex_full_pipeline_with_eq_tags() {
        // End-to-end: eq tags → strip_voice_tags → latex_to_plain_text.
        let input = r#"<voice name="alloy">See <eq latex="\pi">pi</eq> here</voice>"#;
        let after_voice = strip_voice_tags(input);
        let result = latex_to_plain_text(&after_voice);
        assert_eq!(result, "See \u{03C0} here");
    }

    #[test]
    fn test_latex_arrows() {
        assert_eq!(latex_to_plain_text(r"$a \rightarrow b$"), "a \u{2192} b");
        assert_eq!(latex_to_plain_text(r"$a \Rightarrow b$"), "a \u{21D2} b");
    }

    #[test]
    fn test_latex_set_notation() {
        let result = latex_to_plain_text(r"$A \cup B \cap C$");
        assert!(
            result.contains('\u{222A}'),
            "should contain union: {result}"
        );
        assert!(
            result.contains('\u{2229}'),
            "should contain intersection: {result}"
        );
    }

    #[test]
    fn test_latex_subscript_braced() {
        assert_eq!(latex_to_plain_text(r"$x_{10}$"), "x\u{2081}\u{2080}");
    }

    #[test]
    fn test_latex_superscript_braced() {
        assert_eq!(latex_to_plain_text(r"$x^{23}$"), "x\u{00B2}\u{00B3}");
    }

    #[test]
    fn test_latex_newline_not_treated_as_math() {
        // $ on different lines — not inline math.
        let input = "cost is $5\nand $10";
        let result = latex_to_plain_text(input);
        assert_eq!(result, "cost is $5\nand $10");
    }
}
