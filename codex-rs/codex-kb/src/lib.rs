pub mod card;
pub mod error;
pub mod index;
pub mod suggest;
pub mod tool_specs;
pub mod topic;

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use serde::Serialize;
use tokio::sync::Mutex;

use card::Card;
use card::CardFigure;
use card::CardFrontmatter;
use card::CardSource;
use card::parse_card;
use card::serialize_card;
use card::validate_card;
use error::Result;
use index::KbIndex;

const RESEARCH_CONTEXT_FILE: &str = "research-context.md";
const RESEARCH_JOURNAL_FILE: &str = "research-journal.md";

const RESEARCH_CONTEXT_TEMPLATE: &str = "\
## Research Context

### Project

### Priorities

### Not Interested In

### Framings That Work

### Key Decisions Made
";

const RESEARCH_JOURNAL_TEMPLATE: &str = "\
# Research Journal
";

/// Result returned from `write_card`.
#[derive(Debug, Serialize)]
pub struct WriteCardResult {
    pub id: String,
    pub path: String,
    pub tags: Vec<String>,
    pub topics_updated: Vec<String>,
}

/// Result returned from `write_file`.
#[derive(Debug, Serialize)]
pub struct WriteFileResult {
    pub path: String,
}

/// Result returned from `read_file`.
#[derive(Debug, Serialize)]
pub struct ReadFileResult {
    pub path: String,
    pub content: String,
}

/// Summary of a card (returned by search and list).
#[derive(Debug, Serialize)]
pub struct CardSummary {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub capsule: Option<String>,
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// Search results.
#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub query: String,
    pub matches: Vec<CardSummary>,
    pub total_cards: usize,
}

/// Full card read result.
#[derive(Debug, Serialize)]
pub struct ReadCardResult {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub capsule: Option<String>,
    pub status: Option<String>,
    pub source: Option<card::CardSource>,
    pub tensions: Vec<String>,
    pub supersedes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub figures: Vec<card::CardFigure>,
    pub body: String,
}

/// List cards result.
#[derive(Debug, Serialize)]
pub struct ListCardsResult {
    pub cards: Vec<CardSummary>,
    pub total: usize,
}

/// Result returned from `delete_card`.
#[derive(Debug, Serialize)]
pub struct DeleteCardResult {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub topics_updated: Vec<String>,
}

/// Result returned from `reset`.
#[derive(Debug, Serialize)]
pub struct ResetResult {
    pub cards_removed: usize,
    pub topics_removed: usize,
    pub other_removed: Vec<String>,
}

/// Overall KB status.
#[derive(Debug, Serialize)]
pub struct KbStatus {
    pub kb_path: String,
    pub card_count: usize,
    pub topics: std::collections::BTreeMap<String, index::TopicStatus>,
    pub tag_taxonomy: Vec<String>,
}

/// The main knowledge base handle.
pub struct KnowledgeBase {
    pub kb_path: PathBuf,
    index_lock: Arc<Mutex<()>>,
}

impl KnowledgeBase {
    /// Create a new `KnowledgeBase` handle.
    pub fn new(kb_path: PathBuf) -> Self {
        Self {
            kb_path,
            index_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Initialize the KB directory structure (cards/, topics/, index.json,
    /// research-context.md, research-journal.md).
    pub fn init(&self) -> Result<()> {
        std::fs::create_dir_all(self.kb_path.join("cards"))?;
        std::fs::create_dir_all(self.kb_path.join("topics"))?;
        let index_path = self.kb_path.join("index.json");
        if !index_path.exists() {
            let idx = KbIndex::default();
            idx.write_to(&index_path)?;
        }
        let context_path = self.kb_path.join(RESEARCH_CONTEXT_FILE);
        if !context_path.exists() {
            write_atomically(&context_path, RESEARCH_CONTEXT_TEMPLATE)?;
        }
        let journal_path = self.kb_path.join(RESEARCH_JOURNAL_FILE);
        if !journal_path.exists() {
            write_atomically(&journal_path, RESEARCH_JOURNAL_TEMPLATE)?;
        }
        Ok(())
    }

    /// Write (create or update) a card.
    #[allow(clippy::too_many_arguments)]
    pub async fn write_card(
        &self,
        id: String,
        title: String,
        tags: Vec<String>,
        capsule: Option<String>,
        source_type: Option<String>,
        source_refs: Option<Vec<String>>,
        status: Option<String>,
        tensions: Option<Vec<String>>,
        supersedes: Option<Vec<String>>,
        figures: Option<Vec<CardFigure>>,
        contributed_by: Option<String>,
        body: String,
    ) -> Result<WriteCardResult> {
        let today = Utc::now().date_naive();

        let source = source_type.map(|st| CardSource {
            source_type: st,
            refs: source_refs.unwrap_or_default(),
        });

        // Check if the card already exists to preserve date_added.
        let card_path = self.kb_path.join("cards").join(format!("{id}.md"));
        let date_added = if card_path.exists() {
            let existing_content = std::fs::read_to_string(&card_path)?;
            parse_card(&existing_content)
                .ok()
                .and_then(|c| c.frontmatter.date_added)
                .unwrap_or(today)
        } else {
            today
        };

        let frontmatter = CardFrontmatter {
            id: id.clone(),
            title,
            tags: tags.clone(),
            capsule,
            source,
            status,
            tensions: tensions.unwrap_or_default(),
            supersedes: supersedes.unwrap_or_default(),
            figures: figures.unwrap_or_default(),
            date_added: Some(date_added),
            date_updated: Some(today),
            contributed_by,
        };

        let card = Card {
            frontmatter: frontmatter.clone(),
            body,
        };
        validate_card(&card)?;

        let serialized = serialize_card(&card)?;
        self.init()?;

        // Atomic write of card file.
        write_atomically(&card_path, &serialized)?;

        // Acquire lock, update index and topic overviews.
        let _guard = self.index_lock.lock().await;
        let index_path = self.kb_path.join("index.json");
        let mut idx = KbIndex::read_from(&index_path)?;
        idx.register_tags(&tags);

        let mut topics_updated = Vec::new();
        for tag in &tags {
            idx.increment_staleness(tag);
            topic::upsert_card_in_overview(&self.kb_path, tag, &frontmatter)?;
            topics_updated.push(tag.clone());
        }

        idx.write_to(&index_path)?;

        Ok(WriteCardResult {
            id,
            path: card_path.display().to_string(),
            tags,
            topics_updated,
        })
    }

    /// Overwrite the OVERVIEW.md for a topic and reset staleness.
    pub async fn write_overview(&self, topic_name: &str, content: &str) -> Result<()> {
        self.init()?;
        topic::write_overview(&self.kb_path, topic_name, content)?;

        let _guard = self.index_lock.lock().await;
        let index_path = self.kb_path.join("index.json");
        let mut idx = KbIndex::read_from(&index_path)?;
        let today = Utc::now().date_naive();
        idx.reset_staleness(topic_name, today);
        idx.write_to(&index_path)?;
        Ok(())
    }

    /// Delete a card by ID.
    ///
    /// Removes the card file, cleans up its rows from topic overviews,
    /// and increments staleness on affected topics.
    pub async fn delete_card(&self, id: &str) -> Result<DeleteCardResult> {
        let card_path = self.kb_path.join("cards").join(format!("{id}.md"));
        if !card_path.exists() {
            return Err(error::KbError::CardNotFound(id.to_string()));
        }

        // Read card to get metadata before deleting.
        let contents = std::fs::read_to_string(&card_path)?;
        let card = parse_card(&contents)?;
        let title = card.frontmatter.title.clone();
        let tags = card.frontmatter.tags.clone();

        // Delete the card file.
        std::fs::remove_file(&card_path)?;

        // Clean up topic overviews and update index.
        let _guard = self.index_lock.lock().await;
        let index_path = self.kb_path.join("index.json");
        let mut idx = KbIndex::read_from(&index_path)?;

        let mut topics_updated = Vec::new();
        for tag in &tags {
            topic::remove_card_from_overview(&self.kb_path, tag, id)?;
            idx.increment_staleness(tag);
            topics_updated.push(tag.clone());
        }

        idx.write_to(&index_path)?;

        Ok(DeleteCardResult {
            id: id.to_string(),
            title,
            tags,
            topics_updated,
        })
    }

    /// Suggest tags based on title and capsule against the existing taxonomy.
    pub async fn suggest_tags(&self, title: &str, capsule: Option<&str>) -> Result<Vec<String>> {
        let index_path = self.kb_path.join("index.json");
        let idx = KbIndex::read_from(&index_path)?;
        Ok(suggest::suggest_tags(title, capsule, &idx.tag_taxonomy))
    }

    /// Multi-pass search across all cards, ranked by relevance.
    ///
    /// Search strategy (like how you'd use ripgrep iteratively):
    /// 1. Try query as card ID (exact match)
    /// 2. Try query as tag name (exact match)
    /// 3. Score each card across multiple fields with weights:
    ///    - Title match (weight 10 per term)
    ///    - Tag match (weight 8 per term)
    ///    - Capsule match (weight 5 per term)
    ///    - Source refs match (weight 4 per term)
    ///    - Body match (weight 1 per term)
    /// 4. Cards with ANY term matching are included (not all-or-nothing)
    /// 5. Results sorted by score descending
    pub async fn search(
        &self,
        query: &str,
        tag: Option<&str>,
        status_filter: Option<&str>,
        limit: Option<usize>,
    ) -> Result<SearchResult> {
        let limit = limit.unwrap_or(20);
        let cards_dir = self.kb_path.join("cards");
        let total_cards = count_cards(&self.kb_path)?;

        if !cards_dir.exists() {
            return Ok(SearchResult {
                query: query.to_string(),
                matches: Vec::new(),
                total_cards: 0,
            });
        }

        let query_lower = query.to_lowercase();
        // Split into terms on whitespace and common delimiters
        let query_terms: Vec<String> = query_lower
            .split(|c: char| c.is_whitespace() || c == '-' || c == '_' || c == ':')
            .filter(|t| !t.is_empty())
            .map(String::from)
            .collect();

        // Also keep the original hyphenated/compound terms for exact matching
        let query_compounds: Vec<String> = query_lower
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .map(String::from)
            .collect();

        let mut scored: Vec<(CardSummary, u32)> = Vec::new();

        for entry in std::fs::read_dir(&cards_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let contents = std::fs::read_to_string(&path)?;
            let c = match parse_card(&contents) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Apply hard filters.
            if let Some(tag_filter) = tag
                && !c.frontmatter.tags.iter().any(|t| t == tag_filter)
            {
                continue;
            }
            if let Some(sf) = status_filter
                && c.frontmatter.status.as_deref() != Some(sf)
            {
                continue;
            }

            // --- Multi-field scoring ---
            let title_lower = c.frontmatter.title.to_lowercase();
            let id_lower = c.frontmatter.id.to_lowercase();
            let tags_joined = c.frontmatter.tags.join(" ").to_lowercase();
            let capsule_lower = c
                .frontmatter
                .capsule
                .as_deref()
                .unwrap_or("")
                .to_lowercase();
            let source_refs_joined = c
                .frontmatter
                .source
                .as_ref()
                .map(|s| s.refs.join(" "))
                .unwrap_or_default()
                .to_lowercase();
            let body_lower = c.body.to_lowercase();

            let mut score: u32 = 0;

            // Pass 1: exact ID match (highest priority)
            for compound in &query_compounds {
                if id_lower == *compound || id_lower.contains(compound.as_str()) {
                    score += 20;
                }
            }

            // Pass 2: score individual terms across fields
            for term in &query_terms {
                if title_lower.contains(term.as_str()) {
                    score += 10;
                }
                if tags_joined.contains(term.as_str()) {
                    score += 8;
                }
                if capsule_lower.contains(term.as_str()) {
                    score += 5;
                }
                if source_refs_joined.contains(term.as_str()) {
                    score += 4;
                }
                if body_lower.contains(term.as_str()) {
                    score += 1;
                }
            }

            // Pass 3: bonus for compound phrase matches (multi-word queries)
            if query_compounds.len() > 1 {
                let full_query = &query_lower;
                if title_lower.contains(full_query.as_str()) {
                    score += 15;
                }
                if capsule_lower.contains(full_query.as_str()) {
                    score += 8;
                }
            }

            if score == 0 {
                continue;
            }

            // Find best context line (prefer title/capsule, then body)
            let first_term = query_terms.first().map(String::as_str).unwrap_or("");
            let best_context = if title_lower.contains(first_term) {
                Some(format!("title: {}", c.frontmatter.title))
            } else if capsule_lower.contains(first_term) {
                c.frontmatter
                    .capsule
                    .clone()
                    .map(|cap| format!("capsule: {cap}"))
            } else {
                c.body
                    .lines()
                    .find(|line| {
                        let ll = line.to_lowercase();
                        query_terms.iter().any(|t| ll.contains(t.as_str()))
                    })
                    .map(|l| l.trim().to_string())
            };

            scored.push((
                CardSummary {
                    id: c.frontmatter.id,
                    title: c.frontmatter.title,
                    tags: c.frontmatter.tags,
                    capsule: c.frontmatter.capsule,
                    status: c.frontmatter.status,
                    context: best_context,
                },
                score,
            ));
        }

        // Sort by score descending
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored.truncate(limit);

        let matches = scored.into_iter().map(|(summary, _)| summary).collect();

        Ok(SearchResult {
            query: query.to_string(),
            matches,
            total_cards,
        })
    }

    /// Read a card by ID, returning the full content.
    pub async fn read_card(&self, id: &str) -> Result<ReadCardResult> {
        let card_path = self.kb_path.join("cards").join(format!("{id}.md"));
        if !card_path.exists() {
            return Err(error::KbError::CardNotFound(id.to_string()));
        }
        let contents = std::fs::read_to_string(&card_path)?;
        let c = parse_card(&contents)?;
        Ok(ReadCardResult {
            id: c.frontmatter.id,
            title: c.frontmatter.title,
            tags: c.frontmatter.tags,
            capsule: c.frontmatter.capsule,
            status: c.frontmatter.status,
            source: c.frontmatter.source,
            tensions: c.frontmatter.tensions,
            supersedes: c.frontmatter.supersedes,
            figures: c.frontmatter.figures,
            body: c.body,
        })
    }

    /// List all cards, optionally filtered by tag or status.
    pub async fn list_cards(
        &self,
        tag: Option<&str>,
        status_filter: Option<&str>,
        limit: Option<usize>,
    ) -> Result<ListCardsResult> {
        let limit = limit.unwrap_or(50);
        let cards_dir = self.kb_path.join("cards");

        if !cards_dir.exists() {
            return Ok(ListCardsResult {
                cards: Vec::new(),
                total: 0,
            });
        }

        let mut cards = Vec::new();
        let mut total = 0;

        for entry in std::fs::read_dir(&cards_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                let contents = std::fs::read_to_string(&path)?;
                if let Ok(c) = parse_card(&contents) {
                    total += 1;

                    if let Some(tag_filter) = tag
                        && !c.frontmatter.tags.iter().any(|t| t == tag_filter)
                    {
                        continue;
                    }
                    if let Some(sf) = status_filter
                        && c.frontmatter.status.as_deref() != Some(sf)
                    {
                        continue;
                    }

                    if cards.len() < limit {
                        cards.push(CardSummary {
                            id: c.frontmatter.id,
                            title: c.frontmatter.title,
                            tags: c.frontmatter.tags,
                            capsule: c.frontmatter.capsule,
                            status: c.frontmatter.status,
                            context: None,
                        });
                    }
                }
            }
        }

        Ok(ListCardsResult { cards, total })
    }

    /// Get the overall KB status.
    pub async fn status(&self) -> Result<KbStatus> {
        self.init()?;
        let index_path = self.kb_path.join("index.json");
        let idx = KbIndex::read_from(&index_path)?;

        let card_count = count_cards(&self.kb_path)?;

        Ok(KbStatus {
            kb_path: self.kb_path.to_string_lossy().into_owned(),
            card_count,
            topics: idx.topics,
            tag_taxonomy: idx.tag_taxonomy.into_iter().collect(),
        })
    }

    /// Reset the entire knowledge base, removing all cards, topics, assets,
    /// and other generated content. Re-initializes the empty directory structure.
    pub async fn reset(&self) -> Result<ResetResult> {
        let _guard = self.index_lock.lock().await;

        let mut cards_removed = 0;
        let mut topics_removed = 0;
        let mut other_removed = Vec::new();

        // Count cards before removing.
        let cards_dir = self.kb_path.join("cards");
        if cards_dir.exists() {
            cards_removed = std::fs::read_dir(&cards_dir)?
                .filter_map(std::result::Result::ok)
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
                .count();
        }

        // Count topics before removing.
        let topics_dir = self.kb_path.join("topics");
        if topics_dir.exists() {
            topics_removed = std::fs::read_dir(&topics_dir)?
                .filter_map(std::result::Result::ok)
                .filter(|e| e.path().is_dir())
                .count();
        }

        // Remove everything inside the KB directory (but keep the root itself).
        for entry in std::fs::read_dir(&self.kb_path)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if entry.path().is_dir() {
                std::fs::remove_dir_all(entry.path())?;
                if name_str != "cards" && name_str != "topics" {
                    other_removed.push(name_str.into_owned());
                }
            } else {
                std::fs::remove_file(entry.path())?;
                if name_str != "index.json" {
                    other_removed.push(name_str.into_owned());
                }
            }
        }

        // Re-initialize empty structure.
        drop(_guard);
        self.init()?;

        Ok(ResetResult {
            cards_removed,
            topics_removed,
            other_removed,
        })
    }

    /// Write an arbitrary file under the KB root.
    ///
    /// `relative_path` must not contain `..` or start with `/`.
    /// Parent directories are created automatically.
    pub async fn write_file(&self, relative_path: &str, content: &str) -> Result<WriteFileResult> {
        validate_relative_path(relative_path)?;
        let full_path = self.kb_path.join(relative_path);
        write_atomically(&full_path, content)?;
        Ok(WriteFileResult {
            path: full_path.to_string_lossy().into_owned(),
        })
    }

    /// Read an arbitrary file under the KB root.
    ///
    /// `relative_path` must not contain `..` or start with `/`.
    pub async fn read_file(&self, relative_path: &str) -> Result<ReadFileResult> {
        validate_relative_path(relative_path)?;
        let full_path = self.kb_path.join(relative_path);
        let content = std::fs::read_to_string(&full_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                error::KbError::Internal(format!("file not found: {relative_path}"))
            } else {
                error::KbError::Io(e)
            }
        })?;
        Ok(ReadFileResult {
            path: full_path.to_string_lossy().into_owned(),
            content,
        })
    }
}

fn validate_relative_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(error::KbError::Internal("path must not be empty".into()));
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(error::KbError::Internal(
            "path must be relative (no leading slash)".into(),
        ));
    }
    if path.contains("..") {
        return Err(error::KbError::Internal(
            "path must not contain '..' traversal".into(),
        ));
    }
    Ok(())
}

fn write_atomically(path: &Path, contents: &str) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path {} has no parent directory", path.display()),
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let tmp = tempfile::NamedTempFile::new_in(parent)?;
    std::fs::write(tmp.path(), contents)?;
    tmp.persist(path)?;
    Ok(())
}

fn count_cards(kb_path: &Path) -> Result<usize> {
    let cards_dir = kb_path.join("cards");
    if !cards_dir.exists() {
        return Ok(0);
    }
    let count = std::fs::read_dir(&cards_dir)?
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "md"))
        .count();
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn init_creates_structure() {
        let dir = tempfile::tempdir().unwrap();
        let kb = KnowledgeBase::new(dir.path().to_path_buf());
        kb.init().unwrap();

        assert!(dir.path().join("cards").is_dir());
        assert!(dir.path().join("topics").is_dir());
        assert!(dir.path().join("index.json").is_file());
        assert!(dir.path().join("research-context.md").is_file());
        assert!(dir.path().join("research-journal.md").is_file());
    }

    #[tokio::test]
    async fn write_card_creates_card_and_updates_index() {
        let dir = tempfile::tempdir().unwrap();
        let kb = KnowledgeBase::new(dir.path().to_path_buf());

        let result = kb
            .write_card(
                "latent-diffusion".into(),
                "Latent Diffusion Models".into(),
                vec!["diffusion".into(), "generative".into()],
                Some("Diffusion in latent space.".into()),
                Some("paper".into()),
                Some(vec!["arxiv:2112.10752".into()]),
                Some("current".into()),
                None,
                None,
                None,
                Some("test".into()),
                "## Summary\nLatent diffusion.\n".into(),
            )
            .await
            .unwrap();

        assert_eq!(result.id, "latent-diffusion");
        assert_eq!(result.tags, vec!["diffusion", "generative"]);

        // Card file exists and is parseable.
        let card_path = dir.path().join("cards/latent-diffusion.md");
        assert!(card_path.exists());
        let content = std::fs::read_to_string(&card_path).unwrap();
        let card = card::parse_card(&content).unwrap();
        assert_eq!(card.frontmatter.id, "latent-diffusion");

        // Index was updated.
        let idx = KbIndex::read_from(&dir.path().join("index.json")).unwrap();
        assert!(idx.tag_taxonomy.contains("diffusion"));
        assert!(idx.tag_taxonomy.contains("generative"));
        assert_eq!(idx.topics["diffusion"].cards_since_regen, 1);

        // Topic overviews were created.
        let overview =
            std::fs::read_to_string(dir.path().join("topics/diffusion/OVERVIEW.md")).unwrap();
        assert!(overview.contains("Latent Diffusion"));
    }

    #[tokio::test]
    async fn write_card_rejects_invalid_id() {
        let dir = tempfile::tempdir().unwrap();
        let kb = KnowledgeBase::new(dir.path().to_path_buf());

        let result = kb
            .write_card(
                "Bad_Id".into(),
                "Test".into(),
                vec![],
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                "body".into(),
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            error::KbError::InvalidCardId(_)
        ));
    }

    #[tokio::test]
    async fn write_overview_resets_staleness() {
        let dir = tempfile::tempdir().unwrap();
        let kb = KnowledgeBase::new(dir.path().to_path_buf());

        // Write a card to create staleness.
        kb.write_card(
            "test-card".into(),
            "Test".into(),
            vec!["mytopic".into()],
            None,
            None,
            None,
            Some("stub".into()),
            None,
            None,
            None,
            None,
            "body".into(),
        )
        .await
        .unwrap();

        // Overview regen resets staleness.
        kb.write_overview("mytopic", "# Custom overview\n")
            .await
            .unwrap();

        let idx = KbIndex::read_from(&dir.path().join("index.json")).unwrap();
        assert_eq!(idx.topics["mytopic"].cards_since_regen, 0);
        assert!(idx.topics["mytopic"].last_regen.is_some());
    }

    #[tokio::test]
    async fn status_returns_card_count() {
        let dir = tempfile::tempdir().unwrap();
        let kb = KnowledgeBase::new(dir.path().to_path_buf());

        kb.write_card(
            "card-one".into(),
            "Card One".into(),
            vec!["test".into()],
            None,
            None,
            None,
            Some("stub".into()),
            None,
            None,
            None,
            None,
            "body".into(),
        )
        .await
        .unwrap();

        let status = kb.status().await.unwrap();
        assert_eq!(status.card_count, 1);
        assert!(status.tag_taxonomy.contains(&"test".to_string()));
    }

    #[tokio::test]
    async fn suggest_tags_uses_taxonomy() {
        let dir = tempfile::tempdir().unwrap();
        let kb = KnowledgeBase::new(dir.path().to_path_buf());

        // Write a card to populate taxonomy.
        kb.write_card(
            "card-one".into(),
            "Card One".into(),
            vec!["diffusion".into(), "generative".into()],
            None,
            None,
            None,
            Some("stub".into()),
            None,
            None,
            None,
            None,
            "body".into(),
        )
        .await
        .unwrap();

        let suggestions = kb
            .suggest_tags(
                "Diffusion Model Architecture",
                Some("A generative approach"),
            )
            .await
            .unwrap();
        assert!(suggestions.contains(&"diffusion".to_string()));
        assert!(suggestions.contains(&"generative".to_string()));
    }

    #[tokio::test]
    async fn delete_card_removes_file_and_cleans_overviews() {
        let dir = tempfile::tempdir().unwrap();
        let kb = KnowledgeBase::new(dir.path().to_path_buf());

        kb.write_card(
            "to-delete".into(),
            "Card To Delete".into(),
            vec!["topica".into(), "topicb".into()],
            Some("Will be removed.".into()),
            None,
            None,
            Some("stub".into()),
            None,
            None,
            None,
            None,
            "body".into(),
        )
        .await
        .unwrap();

        // Card file and overview rows exist.
        assert!(dir.path().join("cards/to-delete.md").exists());
        let overview_a =
            std::fs::read_to_string(dir.path().join("topics/topica/OVERVIEW.md")).unwrap();
        assert!(overview_a.contains("to-delete"));

        let result = kb.delete_card("to-delete").await.unwrap();
        assert_eq!(result.id, "to-delete");
        assert_eq!(result.title, "Card To Delete");
        assert_eq!(result.tags, vec!["topica", "topicb"]);

        // Card file is gone.
        assert!(!dir.path().join("cards/to-delete.md").exists());

        // Overview rows are cleaned up.
        let overview_a =
            std::fs::read_to_string(dir.path().join("topics/topica/OVERVIEW.md")).unwrap();
        assert!(!overview_a.contains("to-delete"));
        let overview_b =
            std::fs::read_to_string(dir.path().join("topics/topicb/OVERVIEW.md")).unwrap();
        assert!(!overview_b.contains("to-delete"));

        // Card count is 0.
        let status = kb.status().await.unwrap();
        assert_eq!(status.card_count, 0);
    }

    #[tokio::test]
    async fn reset_clears_everything() {
        let dir = tempfile::tempdir().unwrap();
        let kb = KnowledgeBase::new(dir.path().to_path_buf());

        // Write two cards with different topics.
        kb.write_card(
            "card-a".into(),
            "Card A".into(),
            vec!["alpha".into()],
            Some("First card.".into()),
            None,
            None,
            Some("current".into()),
            None,
            None,
            None,
            None,
            "body a".into(),
        )
        .await
        .unwrap();

        kb.write_card(
            "card-b".into(),
            "Card B".into(),
            vec!["beta".into()],
            Some("Second card.".into()),
            None,
            None,
            Some("stub".into()),
            None,
            None,
            None,
            None,
            "body b".into(),
        )
        .await
        .unwrap();

        // Write an extra file (simulating an asset / explanation).
        kb.write_file("assets/fig.png", "fake-image-data")
            .await
            .unwrap();
        kb.write_file("explanations/report.md", "# Report")
            .await
            .unwrap();

        // Verify things exist before reset.
        assert_eq!(kb.status().await.unwrap().card_count, 2);
        assert!(dir.path().join("assets/fig.png").exists());
        assert!(dir.path().join("explanations/report.md").exists());

        // Reset.
        let result = kb.reset().await.unwrap();
        assert_eq!(result.cards_removed, 2);
        assert_eq!(result.topics_removed, 2);
        assert!(result.other_removed.contains(&"assets".to_string()));
        assert!(result.other_removed.contains(&"explanations".to_string()));

        // Verify the KB is empty but structure is intact.
        let status = kb.status().await.unwrap();
        assert_eq!(status.card_count, 0);
        assert!(status.topics.is_empty());
        assert!(status.tag_taxonomy.is_empty());
        assert!(dir.path().join("cards").is_dir());
        assert!(dir.path().join("topics").is_dir());
        assert!(dir.path().join("index.json").is_file());
        assert!(dir.path().join("research-context.md").is_file());
        assert!(dir.path().join("research-journal.md").is_file());

        // Old content is gone.
        assert!(!dir.path().join("assets").exists());
        assert!(!dir.path().join("explanations").exists());
    }

    #[tokio::test]
    async fn delete_card_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let kb = KnowledgeBase::new(dir.path().to_path_buf());
        kb.init().unwrap();

        let result = kb.delete_card("nonexistent").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            error::KbError::CardNotFound(_)
        ));
    }
}
