//! Rendering helpers for the document reader view.

use crate::markdown::append_markdown;
use ratatui::style::Modifier;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;

/// Render a section heading and body into styled lines.
///
/// When `recently_updated` is true, the heading is rendered in green to
/// indicate that the section content was just refreshed by the agent.
pub(super) fn render_section(
    heading: &str,
    content: &str,
    width: u16,
    recently_updated: bool,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Section heading.
    if !heading.is_empty() {
        if recently_updated {
            lines.push(Line::from(vec![
                "✓ ".green(),
                heading.to_string().green().bold(),
            ]));
        } else {
            lines.push(Line::from(heading.to_string().cyan().bold()));
        }
        lines.push(Line::from(""));
    }

    // Section body rendered as markdown.
    if !content.is_empty() {
        let clean = strip_citation_annotations(content);
        let wrap_width = width.saturating_sub(2).max(1) as usize;
        append_markdown(&clean, Some(wrap_width), &mut lines);

        // macOS Terminal.app renders ANSI italic (SGR 3) as reverse video,
        // producing white-bg/black-text on dark terminals.  Replace italic
        // with dim in the reading view to avoid this.
        replace_italic_with_dim(&mut lines);
    }

    if lines.is_empty() {
        lines.push(Line::from("(empty section)".dim().italic()));
    }

    lines
}

/// Count the number of rendered body lines for a content string, accounting
/// for markdown formatting and word wrapping.  Used to map raw content line
/// indices to rendered line positions.
pub(super) fn rendered_body_line_count(content: &str, width: u16) -> usize {
    if content.is_empty() {
        return 0;
    }
    let clean = strip_citation_annotations(content);
    let wrap_width = width.saturating_sub(2).max(1) as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    append_markdown(&clean, Some(wrap_width), &mut lines);
    replace_italic_with_dim(&mut lines);
    lines.len()
}

/// Render a placeholder for a section whose content is still being generated.
///
/// Shows the heading in cyan bold (same as normal) followed by a shimmer
/// animation (when enabled) or a static "Generating..." indicator.
pub(super) fn render_section_loading(
    heading: &str,
    animations_enabled: bool,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    if !heading.is_empty() {
        lines.push(Line::from(heading.to_string().cyan().bold()));
        lines.push(Line::from(""));
    }

    let label = "  \u{25CB} Generating\u{2026}";
    if animations_enabled {
        let shimmer = crate::shimmer::shimmer_spans(label);
        lines.push(Line::from(shimmer));
    } else {
        lines.push(Line::from(label.to_string().dim()));
    }

    lines
}

/// Build the header line with title left-aligned and section nav right-aligned.
///
/// When `streaming_status` is `Some("generating 3/8...")`, it is rendered dim
/// italic after the title (replaces the "thinking..." position).
pub(super) fn header_line(
    title: &str,
    section_num: usize,
    section_count: usize,
    waiting: bool,
    streaming_status: Option<&str>,
    width: u16,
) -> Line<'static> {
    // Account for "│ " + " │" side borders (4 chars).
    let inner_width = (width as usize).saturating_sub(4);

    let status_text = if waiting {
        Some("thinking...")
    } else {
        streaming_status
    };

    let left = if let Some(st) = status_text {
        format!("{title}  {st}")
    } else {
        title.to_string()
    };
    let right = format!("{section_num}/{section_count}");
    let left_width = unicode_width::UnicodeWidthStr::width(left.as_str());
    let right_width = unicode_width::UnicodeWidthStr::width(right.as_str());
    let padding = inner_width
        .saturating_sub(left_width)
        .saturating_sub(right_width);

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push("│ ".dim());
    if let Some(st) = status_text {
        let title_part = format!("{title}  ");
        spans.push(title_part.cyan().bold());
        spans.push(st.to_string().dim().italic());
    } else {
        spans.push(title.to_string().cyan().bold());
    }
    spans.push(Span::from(" ".repeat(padding)));
    spans.push(right.dim());
    spans.push(" │".dim());

    Line::from(spans)
}

/// Build the top border with rounded corners.
pub(super) fn top_border(width: u16) -> Line<'static> {
    let inner = (width as usize).saturating_sub(2);
    Line::from(format!("╭{}╮", "─".repeat(inner)).dim())
}

/// Build the bottom border with rounded corners.
pub(super) fn bottom_border(width: u16) -> Line<'static> {
    let inner = (width as usize).saturating_sub(2);
    Line::from(format!("╰{}╯", "─".repeat(inner)).dim())
}

/// Build a thin horizontal separator.
pub(super) fn separator(width: u16) -> Line<'static> {
    let inner = (width as usize).saturating_sub(2);
    Line::from(format!("├{}┤", "─".repeat(inner)).dim())
}

/// Build a separator with a centered text indicator (e.g. " ▼ scroll for more ").
pub(super) fn separator_with_indicator(width: u16, label: &str) -> Line<'static> {
    let inner = (width as usize).saturating_sub(2);
    let label_width = unicode_width::UnicodeWidthStr::width(label);
    if inner <= label_width {
        // Not enough room for the label — fall back to a plain separator.
        return separator(width);
    }
    let remaining = inner - label_width;
    let left = remaining / 2;
    let right = remaining - left;
    Line::from(vec![
        Span::from(format!("├{}", "─".repeat(left))).dim(),
        Span::from(label.to_string()).dim(),
        Span::from(format!("{}┤", "─".repeat(right))).dim(),
    ])
}

/// Build the keyboard hints line shown below the content area.
pub(super) fn hints_line(
    composer_focused: bool,
    search_focused: bool,
    has_active_search: bool,
    visual_mode: bool,
    width: u16,
) -> Line<'static> {
    let hints: Vec<Span<'static>> = if search_focused {
        vec![
            "Enter".dim().bold(),
            ": search".dim(),
            " | ".dim(),
            "Esc".dim().bold(),
            ": cancel".dim(),
        ]
    } else if composer_focused {
        vec![
            "Tab".dim().bold(),
            ": content".dim(),
            " | ".dim(),
            "Enter".dim().bold(),
            ": send".dim(),
            " | ".dim(),
            "Esc".dim().bold(),
            ": back".dim(),
        ]
    } else if visual_mode {
        vec![
            "hjkl".dim().bold(),
            ": select".dim(),
            " | ".dim(),
            "Tab".dim().bold(),
            ": ask about".dim(),
            " | ".dim(),
            "Esc".dim().bold(),
            ": cancel".dim(),
        ]
    } else if has_active_search {
        vec![
            "n/N".dim().bold(),
            ": match".dim(),
            " | ".dim(),
            "/".dim().bold(),
            ": search".dim(),
            " | ".dim(),
            "Esc".dim().bold(),
            ": clear".dim(),
            " | ".dim(),
            "q".dim().bold(),
            ": done".dim(),
        ]
    } else {
        vec![
            "hjkl".dim().bold(),
            ": move".dim(),
            " | ".dim(),
            "n/p".dim().bold(),
            ": section".dim(),
            " | ".dim(),
            "v".dim().bold(),
            ": select".dim(),
            " | ".dim(),
            "Tab".dim().bold(),
            ": ask".dim(),
            " | ".dim(),
            "q".dim().bold(),
            ": done".dim(),
        ]
    };

    // Wrap in side borders to match the card.
    let hints_width: usize = hints
        .iter()
        .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let pad = (width as usize)
        .saturating_sub(4) // "│ " + " │"
        .saturating_sub(hints_width);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(hints.len() + 4);
    spans.push("│ ".dim());
    spans.extend(hints);
    if pad > 0 {
        spans.push(Span::from(" ".repeat(pad)));
    }
    spans.push(" │".dim());
    Line::from(spans)
}

/// Wrap a content line in card side borders.
///
/// When `updated` is true, the side borders are rendered in green to visually
/// mark the section as recently updated.
pub(super) fn bordered_line(inner: Line<'static>, width: u16, updated: bool) -> Line<'static> {
    let inner_width: usize = inner
        .spans
        .iter()
        .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let pad = (width as usize)
        .saturating_sub(4) // "│ " + " │"
        .saturating_sub(inner_width);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(inner.spans.len() + 4);
    if updated {
        spans.push("│ ".green());
    } else {
        spans.push("│ ".dim());
    }
    spans.extend(inner);
    if pad > 0 {
        spans.push(Span::from(" ".repeat(pad)));
    }
    if updated {
        spans.push(" │".green());
    } else {
        spans.push(" │".dim());
    }
    Line::from(spans)
}

/// Apply selection highlight to a character range `[start_col, end_col)` within
/// a rendered line.  Uses a dark-gray background so it's visually distinct from
/// bold/emphasized text.  Characters outside the range keep their original style.
pub(super) fn apply_char_selection(
    line: Line<'static>,
    start_col: usize,
    end_col: usize,
) -> Line<'static> {
    use ratatui::style::Color;
    use ratatui::style::Style;

    let sel_bg = Color::DarkGray;

    // Clamp end_col to total text length so we never try to allocate
    // huge padding when callers pass usize::MAX.
    let total_len: usize = line.spans.iter().map(|s| s.content.len()).sum();
    let end_col = end_col.min(total_len);
    let start_col = start_col.min(end_col);

    let mut new_spans: Vec<Span<'static>> = Vec::new();
    let mut char_pos = 0usize;

    for span in line.spans {
        let span_text: &str = span.content.as_ref();
        let span_start = char_pos;
        let span_end = span_start + span_text.len();
        let base_style = span.style;

        if span_end <= start_col || span_start >= end_col {
            // Entirely outside selection.
            new_spans.push(span);
        } else {
            // Partially or fully inside selection — split.
            let local_sel_start = start_col.saturating_sub(span_start);
            let local_sel_end = end_col.saturating_sub(span_start).min(span_text.len());

            if local_sel_start > 0 {
                new_spans.push(Span::styled(
                    span_text[..local_sel_start].to_string(),
                    base_style,
                ));
            }
            new_spans.push(Span::styled(
                span_text[local_sel_start..local_sel_end].to_string(),
                Style {
                    bg: Some(sel_bg),
                    ..base_style
                },
            ));
            if local_sel_end < span_text.len() {
                new_spans.push(Span::styled(
                    span_text[local_sel_end..].to_string(),
                    base_style,
                ));
            }
        }
        char_pos = span_end;
    }

    // If the selection extends beyond the text, fill with highlighted spaces.
    if end_col > char_pos && start_col < end_col {
        let extra = end_col - char_pos.max(start_col);
        if extra > 0 {
            new_spans.push(Span::from(" ".repeat(extra)).on_dark_gray());
        }
    }

    Line::from(new_spans)
}

/// Draw side borders ("│") for a range of rows within the card.
///
/// Used by the composer and search bar rendering to add card borders around
/// their content rows.
pub(super) fn draw_side_borders(
    buf: &mut ratatui::buffer::Buffer,
    area_x: u16,
    width: u16,
    start_y: u16,
    row_count: u16,
    bottom: u16,
) {
    for row in 0..row_count {
        let ry = start_y + row;
        if ry < bottom {
            if let Some(cell) = buf.cell_mut((area_x, ry)) {
                cell.set_symbol("│");
                cell.set_style(ratatui::style::Style::default().dim());
            }
            if let Some(cell) = buf.cell_mut((area_x + 1, ry)) {
                cell.set_symbol(" ");
            }
            let rx = area_x + width.saturating_sub(1);
            if let Some(cell) = buf.cell_mut((rx, ry)) {
                cell.set_symbol("│");
                cell.set_style(ratatui::style::Style::default().dim());
            }
            if rx > 0
                && let Some(cell) = buf.cell_mut((rx - 1, ry))
            {
                cell.set_symbol(" ");
            }
        }
    }
}

/// Highlight all occurrences of `query` in a rendered line (case-insensitive).
///
/// Matched substrings are rendered with magenta bold style. The original spans
/// are split at match boundaries so that only the matched characters get the
/// highlight style.
pub(super) fn apply_search_highlights(line: Line<'static>, query: &str) -> Line<'static> {
    if query.is_empty() {
        return line;
    }

    // Flatten all spans into a single string so we can search across span boundaries.
    let full_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let full_lower = full_text.to_lowercase();
    let query_lower = query.to_lowercase();

    // Find all match byte ranges.
    let mut match_ranges: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    while let Some(pos) = full_lower[start..].find(&query_lower) {
        let abs_start = start + pos;
        let abs_end = abs_start + query.len();
        match_ranges.push((abs_start, abs_end));
        start = abs_start + 1;
    }

    if match_ranges.is_empty() {
        return line;
    }

    // Rebuild spans, splitting at match boundaries.
    let mut new_spans: Vec<Span<'static>> = Vec::new();
    let mut byte_cursor = 0usize; // position in full_text
    let mut match_idx = 0;

    for span in line.spans {
        let span_text: &str = span.content.as_ref();
        let span_start = byte_cursor;
        let span_end = span_start + span_text.len();
        let base_style = span.style;

        let mut pos_in_span = 0usize;

        while pos_in_span < span_text.len() && match_idx < match_ranges.len() {
            let (m_start, m_end) = match_ranges[match_idx];

            if m_end <= span_start + pos_in_span {
                // Match is entirely before current position.
                match_idx += 1;
                continue;
            }

            if m_start >= span_end {
                // Match is entirely after this span.
                break;
            }

            // Part before this match (within current span).
            let local_match_start = m_start.saturating_sub(span_start).max(pos_in_span);
            if local_match_start > pos_in_span {
                let before = &span_text[pos_in_span..local_match_start];
                new_spans.push(Span::styled(before.to_string(), base_style));
            }

            // The matched part (within current span).
            let local_match_end = m_end.saturating_sub(span_start).min(span_text.len());
            if local_match_start < local_match_end {
                let matched = &span_text[local_match_start..local_match_end];
                new_spans.push(Span::styled(
                    matched.to_string(),
                    base_style.magenta().bold().underlined(),
                ));
            }

            pos_in_span = local_match_end;

            if span_start + pos_in_span >= m_end {
                match_idx += 1;
            } else {
                break;
            }
        }

        // Remainder of span after all matches.
        if pos_in_span < span_text.len() {
            let rest = &span_text[pos_in_span..];
            new_spans.push(Span::styled(rest.to_string(), base_style));
        }

        byte_cursor = span_end;
    }

    Line::from(new_spans)
}

/// Render a pending-question indicator to append below section content.
///
/// Shows a dashed separator, the user's question, and a "thinking..." line.
pub(super) fn pending_indicator_lines(question: &str, width: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Blank line before separator.
    lines.push(Line::from(""));

    // Light dashed separator.
    let dash_count = (width as usize) / 2;
    let dashes: String = std::iter::repeat_n("\u{2500} ", dash_count)
        .collect::<String>()
        .trim_end()
        .to_string();
    lines.push(Line::from(dashes.dim()));

    // User's question, word-wrapped via textwrap.
    let full_text = format!("You asked: \"{question}\"");
    let wrap_width = width.max(1) as usize;
    for wrapped_line in textwrap::wrap(&full_text, wrap_width) {
        lines.push(Line::from(wrapped_line.into_owned().dim().italic()));
    }

    // Thinking indicator.
    lines.push(Line::from(vec![
        "\u{2022} ".dim(),
        "thinking...".dim().italic(),
    ]));

    lines
}

/// Replace `Modifier::ITALIC` with `Modifier::UNDERLINED` in all spans and
/// line styles.  macOS Terminal.app renders ANSI italic (SGR 3) as reverse
/// video which produces jarring white-bg/black-text on dark terminals.  This
/// is scoped to the reading view only so the shared markdown renderer stays
/// unchanged for upstream compatibility.
/// Strip OpenAI Responses API citation annotations from content.
///
/// The model inserts inline citations delimited by private-use Unicode
/// characters: U+E200 (start), U+E201 (end), U+E202 (separator).
/// These render as boxes/rectangles in terminal fonts.  We strip the
/// entire `\u{e200}...\u{e201}` span plus any surrounding whitespace
/// that would otherwise leave double-spaces or trailing blanks.
pub(super) fn strip_citation_annotations(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{e200}' {
            // Skip everything up to and including the closing \u{e201}.
            for inner in chars.by_ref() {
                if inner == '\u{e201}' {
                    break;
                }
            }
            // Collapse double-space left behind by stripping a citation
            // that was preceded and followed by spaces.
            if out.ends_with(' ') && chars.peek() == Some(&' ') {
                chars.next();
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn replace_italic_with_dim(lines: &mut [Line<'static>]) {
    fn fix_modifiers(add: &mut Modifier, sub: &mut Modifier) {
        if add.contains(Modifier::ITALIC) {
            *add = add.difference(Modifier::ITALIC).union(Modifier::UNDERLINED);
        }
        // Also clear italic from sub_modifier so we don't accidentally
        // remove the replacement that we just added.
        *sub = sub.difference(Modifier::ITALIC);
    }

    for line in lines.iter_mut() {
        fix_modifiers(&mut line.style.add_modifier, &mut line.style.sub_modifier);
        for span in &mut line.spans {
            fix_modifiers(&mut span.style.add_modifier, &mut span.style.sub_modifier);
        }
    }
}
