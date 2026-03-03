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
        // with underline in the reading view to avoid this.
        replace_italic_with_underline(&mut lines);
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
    replace_italic_with_underline(&mut lines);
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
///
/// The right side shows `◀ 3/7: Section Title ▶` with arrow indicators for
/// available navigation directions.  The section heading is truncated if needed
/// to fit the available width.
pub(super) fn header_line(
    title: &str,
    section_num: usize,
    section_count: usize,
    section_heading: &str,
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

    // Build section nav: ◀ 3/7 ▶  (heading is shown in the content area)
    let has_prev = section_num > 1;
    let has_next = section_num < section_count;
    let left_arrow = if has_prev { "◀ " } else { "  " };
    let right_arrow = if has_next { " ▶" } else { "  " };
    let _ = section_heading; // heading already visible in content

    let right = format!("{left_arrow}{section_num}/{section_count}{right_arrow}");
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

/// Truncate a string to fit within `max_width` display columns, appending "…"
/// if it was shortened.
#[allow(dead_code)]
fn truncate_str(s: &str, max_width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    let w = UnicodeWidthStr::width(s);
    if w <= max_width {
        return s.to_string();
    }
    // Leave room for the ellipsis character.
    let budget = max_width.saturating_sub(1);
    let mut result = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw > budget {
            break;
        }
        result.push(ch);
        used += cw;
    }
    result.push('\u{2026}'); // …
    result
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

/// Build a separator with a centered text indicator in a custom style.
pub(super) fn separator_with_indicator_styled(
    width: u16,
    label: &str,
    style: ratatui::style::Style,
) -> Line<'static> {
    let inner = (width as usize).saturating_sub(2);
    let label_width = unicode_width::UnicodeWidthStr::width(label);
    if inner <= label_width {
        return separator(width);
    }
    let remaining = inner - label_width;
    let left = remaining / 2;
    let right = remaining - left;
    Line::from(vec![
        Span::from(format!("├{}", "─".repeat(left))).dim(),
        Span::styled(label.to_string(), style),
        Span::from(format!("{}┤", "─".repeat(right))).dim(),
    ])
}

/// Build the keyboard hints line shown below the content area.
#[allow(clippy::too_many_arguments)]
pub(super) fn hints_line(
    composer_focused: bool,
    search_focused: bool,
    has_active_search: bool,
    visual_mode: bool,
    has_folds: bool,
    pending_quit: bool,
    line_number_input: Option<&str>,
    voice_status: Option<&str>,
    width: u16,
) -> Line<'static> {
    let hints: Vec<Span<'static>> = if let Some(input) = line_number_input {
        vec![
            ":".cyan().bold(),
            Span::from(input.to_string()).cyan(),
            "  (type line number, Enter to jump, Esc to cancel)".dim(),
        ]
    } else if pending_quit {
        vec![
            "Close reading view? ".magenta(),
            "q/y".magenta().bold(),
            ": yes".magenta(),
            " | ".magenta(),
            "any other key".magenta().bold(),
            ": cancel".magenta(),
        ]
    } else if search_focused {
        vec![
            "Enter".dim().bold(),
            ": search".dim(),
            " | ".dim(),
            "Esc".dim().bold(),
            ": cancel".dim(),
        ]
    } else if composer_focused {
        vec![
            "Enter".dim().bold(),
            ": send".dim(),
            " | ".dim(),
            "Esc".dim().bold(),
            ": back to reading".dim(),
        ]
    } else if visual_mode {
        vec![
            "hjkl".dim().bold(),
            ": select".dim(),
            " | ".dim(),
            "Enter".dim().bold(),
            ": explain".dim(),
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
        let mut h: Vec<Span<'static>> = Vec::new();
        h.extend([
            "↑↓/jk".dim().bold(),
            ": scroll".dim(),
            " | ".dim(),
            "n/p".dim().bold(),
            ": section".dim(),
        ]);
        if voice_status.is_some() {
            h.extend([" | ".dim(), "r".dim().bold(), ": read".dim()]);
        }
        if has_folds {
            h.extend([" | ".dim(), "f".dim().bold(), ": fold".dim()]);
        }
        h.extend([
            " | ".dim(),
            "v".dim().bold(),
            ": select".dim(),
            " | ".dim(),
            "Tab".dim().bold(),
            ": ask".dim(),
            " | ".dim(),
            "q".dim().bold(),
            ": close".dim(),
        ]);
        h
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

/// Render a text string inside card borders (for status lines).
pub(super) fn bordered_text_line(text: &str, width: u16) -> Line<'static> {
    let text_width = unicode_width::UnicodeWidthStr::width(text);
    let pad = (width as usize)
        .saturating_sub(4) // "│ " + " │"
        .saturating_sub(text_width);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(4);
    spans.push("│ ".dim());
    spans.push(Span::from(text.to_string()).cyan().bold());
    if pad > 0 {
        spans.push(Span::from(" ".repeat(pad)));
    }
    spans.push(" │".dim());
    Line::from(spans)
}

/// Wrap a content line with side borders and a line number gutter.
///
/// The line number occupies 4 columns (right-aligned) + 1 separator, e.g. `  42│`.
pub(super) fn bordered_line_numbered(
    inner: Line<'static>,
    width: u16,
    updated: bool,
    line_num: usize,
) -> Line<'static> {
    let gutter = format!("{line_num:>4}\u{2502}");
    let gutter_width = 5; // "NNNN│"
    let inner_width: usize = inner
        .spans
        .iter()
        .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let pad = (width as usize)
        .saturating_sub(4) // "│ " + " │"
        .saturating_sub(gutter_width)
        .saturating_sub(inner_width);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(inner.spans.len() + 5);
    if updated {
        spans.push("│ ".green());
    } else {
        spans.push("│ ".dim());
    }
    spans.push(Span::from(gutter).dim());
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

/// Apply bold+underline highlight to a character range within a rendered line.
///
/// Used for word-level karaoke during narration.  The highlighted word gets
/// bold+underline added to its existing style; all other spans keep their
/// original formatting (bold, italic, colors, etc.) untouched.
pub(super) fn apply_word_highlight(
    line: Line<'static>,
    start_col: usize,
    end_col: usize,
) -> Line<'static> {
    use ratatui::style::Modifier;

    let hl = Modifier::BOLD | Modifier::UNDERLINED;
    let mut new_spans: Vec<Span<'static>> = Vec::new();
    let mut char_pos = 0usize;

    for span in line.spans {
        let span_text: &str = span.content.as_ref();
        let span_start = char_pos;
        let span_end = span_start + span_text.len();
        let base_style = span.style;

        if span_end <= start_col || span_start >= end_col {
            // Entirely outside highlight.
            new_spans.push(span);
        } else {
            // Partially or fully inside highlight — split.
            let local_start = start_col.saturating_sub(span_start);
            let local_end = end_col.saturating_sub(span_start).min(span_text.len());

            if local_start > 0 {
                new_spans.push(Span::styled(
                    span_text[..local_start].to_string(),
                    base_style,
                ));
            }
            new_spans.push(Span::styled(
                span_text[local_start..local_end].to_string(),
                base_style.add_modifier(hl),
            ));
            if local_end < span_text.len() {
                new_spans.push(Span::styled(
                    span_text[local_end..].to_string(),
                    base_style,
                ));
            }
        }
        char_pos = span_end;
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

/// Apply fold regions as a post-processing step on rendered lines.
///
/// - **Collapsed** folds replace their line range with a single summary line.
/// - **Expanded** folds prepend a `┊ ` border to each contained line.
///
/// `heading_line_count` is the number of rendered lines consumed by the heading
/// (typically 2: heading text + blank line), used to offset byte-range → line-range
/// mapping since fold byte ranges are relative to the content, not the heading.
pub(super) fn apply_folds(
    lines: Vec<Line<'static>>,
    content: &str,
    heading_line_count: usize,
    width: u16,
    folds: &[super::FoldRegion],
) -> Vec<Line<'static>> {
    if folds.is_empty() {
        return lines;
    }

    // Convert each fold's byte range to a rendered-line range.
    // We render content prefixes through the markdown pipeline to account
    // for word wrapping — a single source line can span multiple rendered
    // lines, so simple newline counting would misalign fold borders.
    struct LineFold {
        start_line: usize,
        end_line: usize, // exclusive
        summary: String,
        collapsed: bool,
    }

    let mut line_folds: Vec<LineFold> = Vec::new();
    for fold in folds {
        let start_rendered =
            rendered_body_line_count(&content[..fold.start.min(content.len())], width);
        let end_rendered = rendered_body_line_count(&content[..fold.end.min(content.len())], width);

        let start_line = heading_line_count + start_rendered;
        let end_line = (heading_line_count + end_rendered).min(lines.len());

        if start_line < end_line {
            line_folds.push(LineFold {
                start_line,
                end_line,
                summary: fold.summary.clone(),
                collapsed: fold.collapsed,
            });
        }
    }

    // Sort by start_line for deterministic processing.
    line_folds.sort_by_key(|f| f.start_line);

    // Build output lines.
    let mut out: Vec<Line<'static>> = Vec::with_capacity(lines.len());
    let mut skip_until: usize = 0;

    for (i, line) in lines.into_iter().enumerate() {
        if i < skip_until {
            continue;
        }

        // Check if a fold starts at this line.
        if let Some(lf) = line_folds.iter().find(|f| f.start_line == i)
            && lf.collapsed
        {
            // Emit a single collapsed summary line.
            out.push(Line::from(vec![
                "┊ ".dim().cyan(),
                "[+] ".dim().cyan(),
                Span::from(lf.summary.clone()).dim().cyan(),
            ]));
            skip_until = lf.end_line;
            continue;
        }

        // Check if an expanded fold starts at this line — emit a header.
        if let Some(lf) = line_folds
            .iter()
            .find(|f| !f.collapsed && f.start_line == i)
        {
            out.push(Line::from(vec![
                "┊ ".dim().cyan(),
                "[-] ".dim().cyan(),
                Span::from(lf.summary.clone()).dim().cyan(),
            ]));
        }

        // Check if this line is inside any expanded fold.
        let in_fold = line_folds
            .iter()
            .any(|f| !f.collapsed && i >= f.start_line && i < f.end_line);

        if in_fold {
            // Prepend fold border to the line.
            let mut spans = vec![Span::from("┊ ").dim().cyan()];
            spans.extend(line.spans);
            out.push(Line::from(spans));
        } else {
            out.push(line);
        }
    }

    out
}

/// Map a pre-fold rendered line index to the corresponding post-fold index.
///
/// Collapsed folds replace N lines with 1 summary line, shifting all
/// subsequent lines up.  This function computes the adjusted index so
/// that highlights (e.g. green "changed" borders) align with the actual
/// post-fold lines returned by [`apply_folds`].
pub(super) fn adjust_line_for_folds(
    pre_fold_line: usize,
    content: &str,
    heading_line_count: usize,
    width: u16,
    folds: &[super::FoldRegion],
) -> usize {
    if folds.is_empty() {
        return pre_fold_line;
    }

    // Build sorted (by start byte) list of collapsed fold line ranges.
    let mut collapsed: Vec<(usize, usize)> = folds
        .iter()
        .filter(|f| f.collapsed)
        .map(|f| {
            let s = rendered_body_line_count(&content[..f.start.min(content.len())], width);
            let e = rendered_body_line_count(&content[..f.end.min(content.len())], width);
            (heading_line_count + s, heading_line_count + e)
        })
        .filter(|(s, e)| e > s)
        .collect();
    collapsed.sort_by_key(|(s, _)| *s);

    let mut adjustment: usize = 0;
    for (fold_start, fold_end) in &collapsed {
        let removed = fold_end - fold_start - 1; // N lines → 1 summary = N-1 removed
        if pre_fold_line > *fold_start {
            if pre_fold_line >= *fold_end {
                adjustment += removed;
            } else {
                // Target is inside the collapsed fold — map to summary line.
                return fold_start.saturating_sub(adjustment);
            }
        }
    }
    pre_fold_line.saturating_sub(adjustment)
}

/// Build the lines for the help overlay showing all keybindings.
/// Build the combined help overlay lines: tutorial intro + keyboard shortcuts.
///
/// When `section_count` is provided, the welcome header shows the section count
/// (used for the first-time tutorial). Otherwise the header is just "Reading View Help".
pub(super) fn help_overlay_lines(width: u16, section_count: Option<usize>) -> Vec<Line<'static>> {
    let inner = (width as usize).saturating_sub(4);
    let mut lines: Vec<Line<'static>> = Vec::new();

    let sep = "─".repeat(inner);
    let push_section = |lines: &mut Vec<Line<'static>>, title: &str| {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            "  ".into(),
            Span::from(title.to_string()).cyan().bold(),
        ]));
        lines.push(Line::from(format!("  {sep}")).dim());
    };

    let push_binding = |lines: &mut Vec<Line<'static>>, keys: &str, desc: &str| {
        let key_col_w = 18;
        let padded_keys = format!("{keys:>key_col_w$}");
        lines.push(Line::from(vec![
            Span::from(padded_keys).bold(),
            "   ".into(),
            Span::from(desc.to_string()).dim(),
        ]));
    };

    // --- Intro section ---
    lines.push(Line::from(""));
    if let Some(count) = section_count {
        lines.push(Line::from(vec![
            "  ".into(),
            "Welcome to Reading View".magenta().bold(),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(
            format!("  Your explanation has {count} sections that you can").dim(),
        ));
        lines.push(Line::from("  navigate through one at a time.".dim()));
    } else {
        lines.push(Line::from(vec![
            "  ".into(),
            "Reading View Help".magenta().bold(),
        ]));
    }

    // --- How it works ---
    push_section(&mut lines, "Getting around");
    lines.push(Line::from(
        "  Use ↑↓ or j/k to scroll within a section".dim(),
    ));
    lines.push(Line::from(
        "  Press n/p to go to the next or previous section".dim(),
    ));

    push_section(&mut lines, "Ask about anything");
    lines.push(Line::from(
        "  Select text with v, then press Enter to explain it".dim(),
    ));
    lines.push(Line::from("  Or press Tab to type your own question".dim()));

    push_section(&mut lines, "Search");
    lines.push(Line::from("  Press / to search within the document".dim()));

    // --- Full keyboard reference ---
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        "  ".into(),
        "All Keyboard Shortcuts".magenta().bold(),
    ]));
    lines.push(Line::from(""));

    push_section(&mut lines, "Navigation");
    push_binding(&mut lines, "j / ↓", "Scroll down one line");
    push_binding(&mut lines, "k / ↑", "Scroll up one line");
    push_binding(&mut lines, "Ctrl+d / Ctrl+u", "Half-page down / up");
    push_binding(&mut lines, "Ctrl+f / Ctrl+b", "Full-page down / up");
    push_binding(&mut lines, "gg", "Jump to top of section");
    push_binding(&mut lines, "G", "Jump to end of section");
    push_binding(&mut lines, "n", "Next section");
    push_binding(&mut lines, "p", "Previous section");

    push_section(&mut lines, "Text Selection");
    push_binding(&mut lines, "v", "Start character selection");
    push_binding(&mut lines, "V", "Start line selection");
    push_binding(&mut lines, "Enter", "Explain selected text");
    push_binding(&mut lines, "Tab", "Ask about selected text");
    push_binding(&mut lines, "Esc", "Cancel selection");

    push_section(&mut lines, "Questions");
    push_binding(&mut lines, "Tab", "Open question composer");
    push_binding(&mut lines, "Enter", "Send question");
    push_binding(&mut lines, "Esc", "Back to reading");

    push_section(&mut lines, "Search");
    push_binding(&mut lines, "/", "Start search");
    push_binding(&mut lines, "n / N", "Next / previous match");
    push_binding(&mut lines, "Esc", "Clear search");

    push_section(&mut lines, "Folds");
    push_binding(&mut lines, "f", "Toggle fold at cursor");
    push_binding(&mut lines, "[ / ]", "Jump to prev / next fold");
    push_binding(&mut lines, "zM / zR", "Collapse / expand all");

    push_section(&mut lines, "Other");
    push_binding(&mut lines, "w / b", "Word forward / backward");
    push_binding(&mut lines, "h / l", "Cursor left / right");
    push_binding(&mut lines, "gx", "Open link at cursor");
    push_binding(&mut lines, "?", "Toggle this help");
    push_binding(&mut lines, "q", "Close reading view");

    lines.push(Line::from(""));
    let dismiss = if section_count.is_some() {
        "  j/k to scroll | q to start reading"
    } else {
        "  j/k to scroll | q to dismiss"
    };
    lines.push(Line::from(dismiss.dim().italic()));
    lines.push(Line::from(""));

    lines
}

fn replace_italic_with_underline(lines: &mut [Line<'static>]) {
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
