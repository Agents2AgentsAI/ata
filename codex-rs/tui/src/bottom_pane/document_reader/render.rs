//! Rendering helpers for the document reader view.

use crate::markdown::append_markdown;
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
        let wrap_width = width.saturating_sub(2).max(1) as usize;
        append_markdown(content, Some(wrap_width), &mut lines);
    }

    if lines.is_empty() {
        lines.push(Line::from("(empty section)".dim().italic()));
    }

    lines
}

/// Build the header line with title left-aligned and section nav right-aligned.
pub(super) fn header_line(
    title: &str,
    section_num: usize,
    section_count: usize,
    waiting: bool,
    width: u16,
) -> Line<'static> {
    let left = if waiting {
        format!(" {title}  thinking...")
    } else {
        format!(" {title}")
    };
    let right = format!(" {section_num}/{section_count} ");
    let padding = (width as usize)
        .saturating_sub(left.len())
        .saturating_sub(right.len());

    let mut spans: Vec<Span<'static>> = Vec::new();
    if waiting {
        // Title part
        let title_part = format!(" {title}  ");
        spans.push(title_part.cyan().bold());
        spans.push("thinking...".dim().italic());
    } else {
        spans.push(format!(" {title}").cyan().bold());
    }
    spans.push(Span::from(" ".repeat(padding)));
    spans.push(right.dim());

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

/// Build the keyboard hints line shown below the content area.
pub(super) fn hints_line(composer_focused: bool, width: u16) -> Line<'static> {
    let hints = if composer_focused {
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
    } else {
        vec![
            "Enter/h/l".dim().bold(),
            ": navigate".dim(),
            " | ".dim(),
            "j/k".dim().bold(),
            ": scroll".dim(),
            " | ".dim(),
            "Tab".dim().bold(),
            ": ask".dim(),
            " | ".dim(),
            "q".dim().bold(),
            ": done".dim(),
        ]
    };

    // Wrap in side borders to match the card.
    let hints_width: usize = hints.iter().map(|s| s.content.len()).sum();
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
