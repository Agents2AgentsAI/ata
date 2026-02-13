use chrono::NaiveDate;
use serde::Deserialize;
use serde::Serialize;

use crate::error::KbError;
use crate::error::Result;

/// A figure attached to a card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardFigure {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

/// Source reference for a card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
}

/// YAML frontmatter for a knowledge card.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CardFrontmatter {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capsule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CardSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub figures: Vec<CardFigure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_added: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_updated: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributed_by: Option<String>,
}

/// A knowledge card: YAML frontmatter + markdown body.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub frontmatter: CardFrontmatter,
    pub body: String,
}

const VALID_STATUSES: &[&str] = &["current", "superseded", "speculative", "stub"];

/// Parse a card from its markdown contents (YAML frontmatter delimited by `---`).
pub fn parse_card(contents: &str) -> Result<Card> {
    let trimmed = contents.trim_start();
    if !trimmed.starts_with("---") {
        return Err(KbError::InvalidFrontmatter(
            "card must start with YAML frontmatter delimited by ---".into(),
        ));
    }

    // Find the closing `---` delimiter.
    let after_open = &trimmed[3..];
    let close_idx = after_open.find("\n---").ok_or_else(|| {
        KbError::InvalidFrontmatter("missing closing --- delimiter for frontmatter".into())
    })?;

    let yaml_str = &after_open[..close_idx];
    let frontmatter: CardFrontmatter = serde_yaml::from_str(yaml_str)?;

    // Body starts after the closing `---` and its trailing newline.
    let body_start = close_idx + 4; // "\n---"
    let body = if body_start < after_open.len() {
        let rest = &after_open[body_start..];
        // Strip up to two leading newlines: one from the `---\n` line ending
        // and one blank line that serialize_card inserts before body.
        let rest = rest.strip_prefix('\n').unwrap_or(rest);
        rest.strip_prefix('\n').unwrap_or(rest)
    } else {
        ""
    };

    Ok(Card {
        frontmatter,
        body: body.to_string(),
    })
}

/// Serialize a card to markdown with YAML frontmatter.
pub fn serialize_card(card: &Card) -> Result<String> {
    let yaml = serde_yaml::to_string(&card.frontmatter)?;
    let mut out = String::with_capacity(yaml.len() + card.body.len() + 16);
    out.push_str("---\n");
    out.push_str(&yaml);
    out.push_str("---\n");
    if !card.body.is_empty() {
        out.push('\n');
        out.push_str(&card.body);
    }
    Ok(out)
}

/// Validate a card's fields.
pub fn validate_card(card: &Card) -> Result<()> {
    validate_card_id(&card.frontmatter.id)?;

    // Tags must be lowercase.
    for tag in &card.frontmatter.tags {
        if tag != &tag.to_lowercase() {
            return Err(KbError::InvalidFrontmatter(format!(
                "tag '{tag}' must be lowercase"
            )));
        }
    }

    // Status must be one of the valid values.
    if let Some(status) = &card.frontmatter.status
        && !VALID_STATUSES.contains(&status.as_str())
    {
        return Err(KbError::InvalidFrontmatter(format!(
            "status '{status}' is not valid; expected one of: {VALID_STATUSES:?}"
        )));
    }

    Ok(())
}

/// Validate that a card ID is kebab-case.
fn validate_card_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(KbError::InvalidCardId(id.to_string()));
    }
    let valid = id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid || id.starts_with('-') || id.ends_with('-') {
        return Err(KbError::InvalidCardId(id.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_card() -> Card {
        Card {
            frontmatter: CardFrontmatter {
                id: "latent-diffusion".into(),
                title: "Latent Diffusion Models".into(),
                tags: vec!["diffusion".into(), "generative".into()],
                capsule: Some("Diffusion in latent space for efficient image generation.".into()),
                source: Some(CardSource {
                    source_type: "paper".into(),
                    refs: vec!["arxiv:2112.10752".into()],
                }),
                status: Some("current".into()),
                tensions: vec![],
                supersedes: vec![],
                figures: vec![],
                date_added: Some(NaiveDate::from_ymd_opt(2025, 1, 15).unwrap_or_default()),
                date_updated: None,
                contributed_by: Some("research-agent".into()),
            },
            body: "## Summary\n\nLatent diffusion applies the diffusion process in a compressed latent space.\n".into(),
        }
    }

    #[test]
    fn roundtrip_parse_serialize() {
        let card = sample_card();
        let serialized = serialize_card(&card).unwrap();
        let parsed = parse_card(&serialized).unwrap();
        assert_eq!(card.frontmatter, parsed.frontmatter);
        assert_eq!(card.body, parsed.body);
    }

    #[test]
    fn validate_accepts_valid_card() {
        let card = sample_card();
        validate_card(&card).unwrap();
    }

    #[test]
    fn validate_rejects_uppercase_tag() {
        let mut card = sample_card();
        card.frontmatter.tags.push("BadTag".into());
        assert!(validate_card(&card).is_err());
    }

    #[test]
    fn validate_rejects_bad_status() {
        let mut card = sample_card();
        card.frontmatter.status = Some("invalid".into());
        assert!(validate_card(&card).is_err());
    }

    #[test]
    fn validate_rejects_bad_id() {
        let mut card = sample_card();
        card.frontmatter.id = "Bad_Id".into();
        assert!(validate_card(&card).is_err());
    }

    #[test]
    fn validate_rejects_empty_id() {
        let mut card = sample_card();
        card.frontmatter.id = String::new();
        assert!(validate_card(&card).is_err());
    }

    #[test]
    fn parse_rejects_missing_frontmatter() {
        assert!(parse_card("no frontmatter here").is_err());
    }

    #[test]
    fn parse_rejects_unclosed_frontmatter() {
        assert!(parse_card("---\nid: test\ntitle: Test\n").is_err());
    }
}
