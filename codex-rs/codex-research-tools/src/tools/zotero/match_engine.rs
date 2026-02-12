use regex::Regex;
use regex::RegexBuilder;

use crate::error::ResearchError;
use crate::error::Result;
use crate::types::ZoteroGrepMatchMode;

const GREP_MATCHER_SIZE_LIMIT: usize = 500_000;
const GREP_MATCHER_DFA_SIZE_LIMIT: usize = 500_000;

#[derive(Debug)]
pub(super) struct GrepMatcher {
    pub(super) regex: Regex,
}

pub(super) fn build_grep_matcher(
    pattern: &str,
    match_mode: &ZoteroGrepMatchMode,
    case_sensitive: bool,
) -> Result<GrepMatcher> {
    let pattern = match match_mode {
        ZoteroGrepMatchMode::Literal => regex::escape(pattern),
        ZoteroGrepMatchMode::Regex => pattern.to_string(),
    };

    let regex = RegexBuilder::new(&pattern)
        .case_insensitive(!case_sensitive)
        .size_limit(GREP_MATCHER_SIZE_LIMIT)
        .dfa_size_limit(GREP_MATCHER_DFA_SIZE_LIMIT)
        .build()
        .map_err(|err| ResearchError::InvalidInput(format!("invalid grep pattern: {err}")))?;

    Ok(GrepMatcher { regex })
}

pub(super) fn strip_html_to_text(input: &str) -> String {
    let mut text = String::with_capacity(input.len());
    let mut in_tag = false;

    for ch in input.chars() {
        match ch {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' => {
                in_tag = false;
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }

    let decoded = decode_basic_html_entities(text.as_str());
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_basic_html_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }

    let mut out = String::with_capacity(value.len());
    let mut remaining = value;

    while let Some(entity_start) = remaining.find('&') {
        out.push_str(&remaining[..entity_start]);
        remaining = &remaining[entity_start..];

        let Some(entity_end) = remaining.find(';') else {
            out.push_str(remaining);
            return out;
        };
        let entity = &remaining[..=entity_end];
        match entity {
            "&nbsp;" | "&#160;" => out.push(' '),
            "&amp;" => out.push('&'),
            "&lt;" => out.push('<'),
            "&gt;" => out.push('>'),
            "&quot;" => out.push('"'),
            "&#39;" => out.push('\''),
            "&mdash;" | "&#8212;" | "&#x2014;" | "&#X2014;" => out.push('\u{2014}'),
            "&ndash;" | "&#8211;" | "&#x2013;" | "&#X2013;" => out.push('\u{2013}'),
            "&rsquo;" | "&#8217;" | "&#x2019;" | "&#X2019;" => out.push('\u{2019}'),
            _ => out.push_str(entity),
        }
        remaining = &remaining[entity_end + 1..];
    }

    out.push_str(remaining);
    out
}
