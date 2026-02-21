use std::path::Path;

use tempfile::NamedTempFile;

use crate::card::CardFrontmatter;
use crate::error::Result;

/// Create the topic directory and scaffold an OVERVIEW.md if it doesn't exist.
pub fn ensure_topic_dir(kb_path: &Path, topic: &str) -> Result<()> {
    let topic_dir = kb_path.join("topics").join(topic);
    std::fs::create_dir_all(&topic_dir)?;

    let overview_path = topic_dir.join("OVERVIEW.md");
    if !overview_path.exists() {
        let title = title_case(topic);
        let scaffold = format!(
            "# Topic: {title}\n\n\
             ## Cards\n\n\
             | Card | Status | Capsule |\n\
             |------|--------|--------|\n\n\
             ## Comparative Analysis\n\
             *Not yet generated.*\n"
        );
        std::fs::write(&overview_path, scaffold)?;
    }
    Ok(())
}

/// Insert or update a card row in the topic's OVERVIEW.md table.
pub fn upsert_card_in_overview(kb_path: &Path, topic: &str, fm: &CardFrontmatter) -> Result<()> {
    ensure_topic_dir(kb_path, topic)?;

    let overview_path = kb_path.join("topics").join(topic).join("OVERVIEW.md");
    let contents = std::fs::read_to_string(&overview_path)?;

    let status = fm.status.as_deref().unwrap_or("stub");
    let capsule = truncate_capsule(fm.capsule.as_deref().unwrap_or(""), 100);
    let new_row = format!(
        "| [{}](../../cards/{}.md) | {status} | {capsule} |",
        fm.title, fm.id
    );

    let link_pattern = format!("cards/{}.md)", fm.id);
    let mut lines: Vec<String> = contents.lines().map(String::from).collect();
    let mut found = false;

    // Find the existing row for this card and replace it.
    for line in &mut lines {
        if line.contains(&link_pattern) {
            *line = new_row.clone();
            found = true;
            break;
        }
    }

    if !found {
        // Insert after the table header separator (the `|---|---|---|` line).
        let insert_pos = lines
            .iter()
            .position(|l| l.starts_with("|---") || l.starts_with("| ---"))
            .map(|i| i + 1);

        if let Some(pos) = insert_pos {
            lines.insert(pos, new_row);
        }
    }

    let output = lines.join("\n");
    // Ensure trailing newline.
    let output = if output.ends_with('\n') {
        output
    } else {
        format!("{output}\n")
    };

    write_atomically(&overview_path, &output)?;
    Ok(())
}

/// Remove a card row from a topic's OVERVIEW.md table.
pub fn remove_card_from_overview(kb_path: &Path, topic: &str, card_id: &str) -> Result<()> {
    let overview_path = kb_path.join("topics").join(topic).join("OVERVIEW.md");
    if !overview_path.exists() {
        return Ok(());
    }

    let contents = std::fs::read_to_string(&overview_path)?;
    let link_pattern = format!("cards/{card_id}.md)");

    let filtered: Vec<&str> = contents
        .lines()
        .filter(|line| !line.contains(&link_pattern))
        .collect();

    let mut output = filtered.join("\n");
    if !output.ends_with('\n') {
        output.push('\n');
    }

    write_atomically(&overview_path, &output)?;
    Ok(())
}

/// Overwrite the OVERVIEW.md entirely (for Tier 2 regen by agent).
pub fn write_overview(kb_path: &Path, topic: &str, content: &str) -> Result<()> {
    ensure_topic_dir(kb_path, topic)?;
    let overview_path = kb_path.join("topics").join(topic).join("OVERVIEW.md");
    write_atomically(&overview_path, content)?;
    Ok(())
}

fn write_atomically(path: &Path, contents: &str) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path {} has no parent directory", path.display()),
        )
    })?;
    let tmp = NamedTempFile::new_in(parent)?;
    std::fs::write(tmp.path(), contents)?;
    tmp.persist(path)?;
    Ok(())
}

fn title_case(s: &str) -> String {
    s.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    format!("{upper}{}", chars.as_str())
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_capsule(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let truncated = &s[..s.floor_char_boundary(max.saturating_sub(3))];
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_case_works() {
        assert_eq!(title_case("diffusion"), "Diffusion");
        assert_eq!(title_case("latent-diffusion"), "Latent Diffusion");
        assert_eq!(title_case("vla"), "Vla");
        assert_eq!(title_case("vq-vae"), "Vq Vae");
        assert_eq!(title_case("foundation-model"), "Foundation Model");
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate_capsule("hello", 100), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let long = "a".repeat(200);
        let result = truncate_capsule(&long, 100);
        assert!(result.len() <= 100);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn ensure_topic_creates_directory_and_scaffold() {
        let dir = tempfile::tempdir().unwrap();
        let kb = dir.path();
        ensure_topic_dir(kb, "diffusion").unwrap();

        let overview = kb.join("topics/diffusion/OVERVIEW.md");
        assert!(overview.exists());

        let content = std::fs::read_to_string(&overview).unwrap();
        assert!(content.contains("# Topic: Diffusion"));
        assert!(content.contains("| Card | Status | Capsule |"));
    }

    #[test]
    fn upsert_card_inserts_and_updates() {
        let dir = tempfile::tempdir().unwrap();
        let kb = dir.path();
        ensure_topic_dir(kb, "diffusion").unwrap();

        let fm = CardFrontmatter {
            id: "latent-diffusion".into(),
            title: "Latent Diffusion".into(),
            tags: vec!["diffusion".into()],
            capsule: Some("Diffusion in latent space.".into()),
            source: None,
            status: Some("current".into()),
            tensions: vec![],
            supersedes: vec![],
            figures: vec![],
            date_added: None,
            date_updated: None,
            contributed_by: None,
        };

        // Insert
        upsert_card_in_overview(kb, "diffusion", &fm).unwrap();
        let content = std::fs::read_to_string(kb.join("topics/diffusion/OVERVIEW.md")).unwrap();
        assert!(content.contains("Latent Diffusion"));
        assert!(content.contains("current"));

        // Update (change status)
        let mut fm2 = fm;
        fm2.status = Some("superseded".into());
        upsert_card_in_overview(kb, "diffusion", &fm2).unwrap();
        let content = std::fs::read_to_string(kb.join("topics/diffusion/OVERVIEW.md")).unwrap();
        assert!(content.contains("superseded"));
        // Should only have one row for this card.
        let count = content.matches("latent-diffusion").count();
        assert_eq!(count, 1);
    }
}
