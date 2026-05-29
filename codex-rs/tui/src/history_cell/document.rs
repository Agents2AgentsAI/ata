//! Transcript cell rendered when the agent presents a document via the
//! reading view. Lives in the history so the user can see what was shown
//! after the reader overlay is dismissed.

use super::*;

#[derive(Debug, Clone)]
pub(crate) struct DocumentCell {
    pub(crate) title: String,
    pub(crate) section_headings: Vec<String>,
    pub(crate) final_content: String,
}

pub(crate) fn new_document_cell(
    title: String,
    section_headings: Vec<String>,
    final_content: String,
) -> DocumentCell {
    DocumentCell {
        title,
        section_headings,
        final_content,
    }
}

impl HistoryCell for DocumentCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let section_count = self.section_headings.len();
        vec![
            vec![
                "\u{2022} ".dim(),
                "Agent showed document: ".dim(),
                self.title.clone().into(),
                format!(" ({section_count} sections)").dim(),
            ]
            .into(),
            "    Ask the agent to reopen it if needed."
                .dim()
                .italic()
                .into(),
        ]
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = self.display_lines(width);
        lines.push(Line::from(""));
        let wrap_width = width.saturating_sub(2).max(1) as usize;
        append_markdown(&self.final_content, Some(wrap_width), None, &mut lines);
        lines
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.display_lines(0)
    }
}
