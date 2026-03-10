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
        let clean = strip_pause_sentinels(&strip_citation_annotations(content));
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
    let clean = strip_pause_sentinels(&strip_citation_annotations(content));
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
    voice_paused: bool,
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
            "q/Esc/y".magenta().bold(),
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
        #[cfg(not(target_os = "linux"))]
        h.extend([" | ".dim(), "r".dim().bold(), ": read".dim()]);
        if voice_status.is_some() {
            if voice_paused {
                h.extend([" | ".dim(), "s".dim().bold(), ": resume".dim()]);
            } else {
                h.extend([" | ".dim(), "s".dim().bold(), ": pause".dim()]);
            }
        }
        h.extend([" | ".dim(), "t".dim().bold(), ": toc".dim()]);
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

    // Guard against invalid / empty range.
    if start_col >= end_col {
        return line;
    }

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
                new_spans.push(Span::styled(span_text[local_end..].to_string(), base_style));
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

/// Strip `[PAUSE:N]` sentinels from content so they don't appear in the
/// rendered reading view.  These markers are used by the TTS pipeline and
/// should be invisible to the user.
fn strip_pause_sentinels(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut remaining = content;
    let marker = "[PAUSE:";
    while let Some(start) = remaining.find(marker) {
        out.push_str(&remaining[..start]);
        remaining = &remaining[start + marker.len()..];
        // Skip past the closing ']'.
        if let Some(end) = remaining.find(']') {
            remaining = &remaining[end + 1..];
        }
    }
    out.push_str(remaining);
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
    push_binding(&mut lines, "t", "Table of contents");
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

/// Build the lines for the Table of Contents overlay.
///
/// Shows all section headings with the current section highlighted.
/// `current_section` is the section being viewed (marked with ▶).
/// `selected_index` is the cursor position in the TOC list.
/// `visited` indicates which sections have been read.
pub(super) fn toc_overlay_lines(
    _width: u16,
    sections: &[(String, bool)],
    current_section: usize,
    selected_index: usize,
    visited: &std::collections::HashSet<usize>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        "  ".into(),
        "Table of Contents".magenta().bold(),
    ]));
    lines.push(Line::from(""));

    for (i, (heading, recently_updated)) in sections.iter().enumerate() {
        let display_heading = if heading.is_empty() {
            "Intro".to_string()
        } else {
            heading.clone()
        };

        let is_selected = i == selected_index;
        let is_current = i == current_section;
        let is_visited = visited.contains(&i);

        let marker = if is_current {
            "\u{25B6} "
        } else if is_visited {
            "\u{2713} "
        } else {
            "  "
        };

        let num = format!("  {marker}{}. ", i + 1);

        if is_selected {
            if *recently_updated {
                lines.push(Line::from(vec![
                    Span::from(num).green().bold(),
                    Span::from(display_heading).green().bold(),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::from(num).cyan().bold(),
                    Span::from(display_heading).cyan().bold(),
                ]));
            }
        } else if *recently_updated {
            lines.push(Line::from(vec![
                Span::from(num).green(),
                Span::from(display_heading).green(),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::from(num).dim(),
                Span::from(display_heading).dim(),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(
        "  j/k to navigate | Enter to jump | t/Esc to dismiss"
            .dim()
            .italic(),
    ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;
    use ratatui::style::Modifier;
    use ratatui::style::Style;
    use ratatui::text::Line;
    use ratatui::text::Span;

    #[test]
    fn highlight_single_word() {
        // "hello world" — highlight cols 0..5 → "hello" is bold+underline.
        let line = Line::from("hello world".to_string());
        let result = apply_word_highlight(line, 0, 5);
        let hl = Modifier::BOLD | Modifier::UNDERLINED;

        assert_eq!(
            result.spans.len(),
            2,
            "should split into highlighted + rest"
        );
        assert_eq!(result.spans[0].content.as_ref(), "hello");
        assert!(
            result.spans[0].style.add_modifier.contains(hl),
            "highlighted span should have BOLD|UNDERLINED"
        );
        assert_eq!(result.spans[1].content.as_ref(), " world");
        assert!(
            !result.spans[1].style.add_modifier.contains(Modifier::BOLD),
            "non-highlighted span should NOT have BOLD"
        );
    }

    #[test]
    fn highlight_cross_span() {
        // Line with two spans: "hello " (cyan) + "world" (green).
        // Highlight cols 3..8 crosses the boundary → splits both spans.
        let line = Line::from(vec![
            Span::styled("hello ".to_string(), Style::default().fg(Color::Cyan)),
            Span::styled("world".to_string(), Style::default().fg(Color::Green)),
        ]);
        let result = apply_word_highlight(line, 3, 8);
        let hl = Modifier::BOLD | Modifier::UNDERLINED;

        // Expected spans: "hel" (cyan, no hl), "lo " (cyan, hl), "wo" (green, hl), "rld" (green, no hl)
        assert_eq!(
            result.spans.len(),
            4,
            "should split into 4 spans at highlight boundaries"
        );
        assert_eq!(result.spans[0].content.as_ref(), "hel");
        assert!(!result.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(result.spans[0].style.fg, Some(Color::Cyan));

        assert_eq!(result.spans[1].content.as_ref(), "lo ");
        assert!(result.spans[1].style.add_modifier.contains(hl));
        assert_eq!(result.spans[1].style.fg, Some(Color::Cyan));

        assert_eq!(result.spans[2].content.as_ref(), "wo");
        assert!(result.spans[2].style.add_modifier.contains(hl));
        assert_eq!(result.spans[2].style.fg, Some(Color::Green));

        assert_eq!(result.spans[3].content.as_ref(), "rld");
        assert!(!result.spans[3].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(result.spans[3].style.fg, Some(Color::Green));
    }

    #[test]
    fn highlight_full_line() {
        let line = Line::from("entire line".to_string());
        let result = apply_word_highlight(line, 0, 11);
        let hl = Modifier::BOLD | Modifier::UNDERLINED;

        assert_eq!(
            result.spans.len(),
            1,
            "entire line highlighted should stay as one span"
        );
        assert_eq!(result.spans[0].content.as_ref(), "entire line");
        assert!(result.spans[0].style.add_modifier.contains(hl));
    }

    #[test]
    fn highlight_preserves_styles() {
        // Span with cyan + bold — after highlight, should be cyan + bold + underline.
        let style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let line = Line::from(Span::styled("styled text".to_string(), style));
        let result = apply_word_highlight(line, 0, 11);

        assert_eq!(result.spans.len(), 1);
        let s = &result.spans[0].style;
        assert_eq!(s.fg, Some(Color::Cyan), "fg color should be preserved");
        assert!(
            s.add_modifier.contains(Modifier::BOLD),
            "existing BOLD should be preserved"
        );
        assert!(
            s.add_modifier.contains(Modifier::UNDERLINED),
            "UNDERLINED should be added"
        );
    }

    #[test]
    fn highlight_zero_width() {
        // start_col == end_col: the highlighted slice is empty so no
        // visible character gets bold/underline, even though the function
        // still splits the span at the boundary.
        let line = Line::from("no change".to_string());
        let result = apply_word_highlight(line, 3, 3);

        // Collect all text to verify content is preserved.
        let full_text: String = result.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(full_text, "no change", "full text should be preserved");

        // The highlighted (empty) span has zero length, so no visible chars
        // receive the modifier.
        let hl = Modifier::BOLD | Modifier::UNDERLINED;
        for span in &result.spans {
            if !span.content.is_empty() {
                assert!(
                    !span.style.add_modifier.contains(hl),
                    "non-empty span should NOT have highlight modifiers",
                );
            }
        }
    }

    // ─── Adversarial tests: apply_word_highlight ────────────────────────

    #[test]
    fn adversarial_highlight_empty_line() {
        // Empty line (no spans) should return empty without panicking.
        let line = Line::from(Vec::<Span<'static>>::new());
        let result = apply_word_highlight(line, 0, 5);
        assert!(
            result.spans.is_empty(),
            "empty line should produce empty result"
        );
    }

    #[test]
    fn adversarial_highlight_start_col_beyond_length() {
        // start_col and end_col both beyond the line length.
        let line = Line::from("short".to_string());
        let result = apply_word_highlight(line, 10, 15);
        // span_end(5) <= start_col(10) -> entirely outside -> original span preserved.
        let full_text: String = result.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(full_text, "short", "text should be preserved");
        // No span should be highlighted.
        let hl = Modifier::BOLD | Modifier::UNDERLINED;
        for span in &result.spans {
            assert!(
                !span.style.add_modifier.contains(hl),
                "no span should be highlighted when range is beyond line"
            );
        }
    }

    #[test]
    fn adversarial_highlight_end_col_beyond_length() {
        // start_col within line, end_col beyond it -- should highlight to end.
        let line = Line::from("hello".to_string());
        let result = apply_word_highlight(line, 3, 100);
        let hl = Modifier::BOLD | Modifier::UNDERLINED;

        // Should split into "hel" (no hl) + "lo" (hl).
        let full_text: String = result.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(full_text, "hello");
        assert_eq!(result.spans.len(), 2);
        assert_eq!(result.spans[0].content.as_ref(), "hel");
        assert!(!result.spans[0].style.add_modifier.contains(hl));
        assert_eq!(result.spans[1].content.as_ref(), "lo");
        assert!(result.spans[1].style.add_modifier.contains(hl));
    }

    #[test]
    fn adversarial_highlight_start_col_greater_than_end_col() {
        // BUG PROBE: start_col > end_col is an invalid range.
        // Should not panic. Ideally should return the line unchanged.
        let line = Line::from("hello world".to_string());
        let result = apply_word_highlight(line, 8, 3);
        // If we get here, at least it didn't panic.
        let full_text: String = result.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(
            full_text, "hello world",
            "text should be preserved even with invalid range"
        );
    }

    #[test]
    fn adversarial_highlight_multibyte_unicode() {
        // The function uses byte offsets (span_text.len() is byte count).
        // The caller (set_voice_reading_progress/WordOffsets) also uses byte offsets.
        // Test that multi-byte chars work when highlight boundaries are at
        // char boundaries.
        //
        // "\u{00E9}" = e-acute (2 bytes in UTF-8)
        let line = Line::from("caf\u{00E9} au lait".to_string());
        // "caf\u{00E9}" is 5 bytes (c=1, a=1, f=1, \u{00E9}=2)
        // Highlight bytes 0..5 should get "caf\u{00E9}"
        let result = apply_word_highlight(line, 0, 5);
        let hl = Modifier::BOLD | Modifier::UNDERLINED;

        let full_text: String = result.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(full_text, "caf\u{00E9} au lait");
        assert_eq!(result.spans[0].content.as_ref(), "caf\u{00E9}");
        assert!(result.spans[0].style.add_modifier.contains(hl));
    }

    #[test]
    fn adversarial_highlight_mid_multibyte_char() {
        // BUG PROBE: What happens when start_col falls in the MIDDLE of a
        // multi-byte character? This would cause a panic on str slicing.
        //
        // "\u{00E9}" is 2 bytes. If start_col=4 (middle of the 2-byte char
        // starting at byte 3), the slice span_text[..4] would panic.
        //
        // In practice, WordOffsets shouldn't produce such offsets, but let's
        // test the robustness of the function itself.
        let _line = Line::from("caf\u{00E9} ok".to_string());
        // byte layout: c(0) a(1) f(2) \u{00E9}(3,4) ' '(5) o(6) k(7)
        // start_col=4 is in the middle of the 2-byte \u{00E9}.
        let result = std::panic::catch_unwind(|| {
            apply_word_highlight(Line::from("caf\u{00E9} ok".to_string()), 4, 7)
        });
        // Document whether this panics or not.
        if result.is_err() {
            // This is expected -- str slicing at non-char-boundary panics.
            // Not a bug in apply_word_highlight per se, since callers should
            // always pass valid byte offsets. But worth documenting.
        } else {
            // If it somehow doesn't panic, that's fine too.
            let line_result = result.expect("already checked");
            let full_text: String = line_result
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect();
            assert!(full_text.contains("ok"));
        }
    }

    #[test]
    fn adversarial_highlight_already_styled_span() {
        // A span that's already bold+underlined -- re-applying should not cause issues.
        let style = Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        let line = Line::from(Span::styled("already styled".to_string(), style));
        let result = apply_word_highlight(line, 0, 14);

        assert_eq!(result.spans.len(), 1);
        let s = &result.spans[0].style;
        // Should still have bold + underlined (idempotent).
        assert!(s.add_modifier.contains(Modifier::BOLD));
        assert!(s.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(s.fg, Some(Color::Red));
    }

    #[test]
    fn adversarial_highlight_empty_span_in_middle() {
        // Line with an empty span in the middle.
        let line = Line::from(vec![
            Span::raw("hello".to_string()),
            Span::raw(String::new()),
            Span::raw(" world".to_string()),
        ]);
        let result = apply_word_highlight(line, 2, 8);
        let hl = Modifier::BOLD | Modifier::UNDERLINED;

        let full_text: String = result.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(full_text, "hello world");
        // The empty span should not cause issues.
        // Check that "llo wo" (bytes 2..8) is highlighted.
        let highlighted_text: String = result
            .spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(hl))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(highlighted_text, "llo wo");
    }

    #[test]
    fn adversarial_highlight_zero_zero() {
        // start_col=0, end_col=0 -- empty highlight at start.
        let line = Line::from("test".to_string());
        let result = apply_word_highlight(line, 0, 0);
        let full_text: String = result.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(full_text, "test");
        // No visible characters should be highlighted.
        let hl = Modifier::BOLD | Modifier::UNDERLINED;
        for span in &result.spans {
            if !span.content.is_empty() {
                assert!(
                    !span.style.add_modifier.contains(hl),
                    "no visible chars should be highlighted with 0..0 range"
                );
            }
        }
    }
}
