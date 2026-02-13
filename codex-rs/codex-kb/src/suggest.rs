use std::collections::BTreeSet;

/// Suggest tags for a card by matching tokenized title+capsule against existing taxonomy.
///
/// Matching strategies (in order):
/// 1. Exact tag appears as substring in text (e.g. "diffusion" in "latent diffusion models")
/// 2. Any text token is a substring of a tag (e.g. token "transformer" matches tag "transformers")
/// 3. Tag segments match text tokens (e.g. tag "vq-vae" splits to ["vq","vae"], both found)
/// 4. Acronym expansion: tag initials match a text token (e.g. tag "vla" matches tokens
///    containing "vision", "language", "action" nearby, or the literal "vla" in text)
pub fn suggest_tags(
    title: &str,
    capsule: Option<&str>,
    taxonomy: &BTreeSet<String>,
) -> Vec<String> {
    if taxonomy.is_empty() {
        return Vec::new();
    }

    let text = match capsule {
        Some(c) => format!("{title} {c}"),
        None => title.to_string(),
    };
    let text_lower = text.to_lowercase();

    // Split on non-alphanumeric (including hyphens) so "vision-language-action"
    // becomes ["vision", "language", "action"] for acronym matching.
    let tokens: Vec<&str> = text_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();

    let mut matches: Vec<String> = taxonomy
        .iter()
        .filter(|tag| {
            let tag_lower = tag.to_lowercase();

            // Strategy 1: tag is a substring of the text
            if text_lower.contains(&tag_lower) {
                return true;
            }

            // Strategy 2: any token (3+ chars) is a substring of the tag
            if tokens
                .iter()
                .any(|token| token.len() >= 3 && tag_lower.contains(token))
            {
                return true;
            }

            // Strategy 3: for hyphenated tags, check if all segments appear as tokens
            if tag_lower.contains('-') {
                let tag_parts: Vec<&str> = tag_lower.split('-').collect();
                if tag_parts.len() >= 2
                    && tag_parts.iter().all(|part| {
                        part.len() >= 3
                            && tokens
                                .iter()
                                .any(|t| t.len() >= 3 && (*t == *part || t.contains(part)))
                    })
                {
                    return true;
                }
            }

            // Strategy 4: short tags (2-5 chars, no hyphens) might be acronyms —
            // check if each letter starts a token in order
            if tag_lower.len() >= 2
                && tag_lower.len() <= 5
                && !tag_lower.contains('-')
                && tag_lower.chars().all(|c| c.is_ascii_alphabetic())
                && acronym_matches_tokens(&tag_lower, &tokens)
            {
                return true;
            }

            false
        })
        .cloned()
        .collect();

    matches.sort();
    matches.dedup();
    matches
}

/// Check if a short tag (potential acronym) matches consecutive token initials.
/// E.g., "vla" matches tokens [..., "vision", "language", "action", ...]
fn acronym_matches_tokens(acronym: &str, tokens: &[&str]) -> bool {
    let chars: Vec<char> = acronym.chars().collect();
    let n = chars.len();

    // Try to find a consecutive run of tokens whose first chars spell the acronym.
    if tokens.len() < n {
        return false;
    }

    for start in 0..=tokens.len() - n {
        let mut all_match = true;
        for (i, &ch) in chars.iter().enumerate() {
            if let Some(first_char) = tokens[start + i].chars().next() {
                if first_char != ch {
                    all_match = false;
                    break;
                }
            } else {
                all_match = false;
                break;
            }
        }
        if all_match {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taxonomy_from(tags: &[&str]) -> BTreeSet<String> {
        tags.iter().map(std::string::ToString::to_string).collect()
    }

    #[test]
    fn suggests_matching_tags() {
        let taxonomy = taxonomy_from(&["diffusion", "generative", "reinforcement-learning"]);
        let tags = suggest_tags(
            "Latent Diffusion Models",
            Some("Efficient generative modeling in latent space"),
            &taxonomy,
        );
        assert!(tags.contains(&"diffusion".to_string()));
        assert!(tags.contains(&"generative".to_string()));
        assert!(!tags.contains(&"reinforcement-learning".to_string()));
    }

    #[test]
    fn empty_taxonomy_returns_empty() {
        let taxonomy = BTreeSet::new();
        let tags = suggest_tags("anything", None, &taxonomy);
        assert!(tags.is_empty());
    }

    #[test]
    fn token_substring_match() {
        let taxonomy = taxonomy_from(&["transformers"]);
        let tags = suggest_tags(
            "Attention Is All You Need",
            Some("A transformer architecture"),
            &taxonomy,
        );
        assert!(tags.contains(&"transformers".to_string()));
    }

    #[test]
    fn acronym_vla_matches_vision_language_action() {
        let taxonomy = taxonomy_from(&["vla", "robotics", "nlp"]);
        let tags = suggest_tags(
            "RT-2: Vision-Language-Action Models",
            Some("Transfers web knowledge to robotic control via VLM fine-tuning"),
            &taxonomy,
        );
        assert!(
            tags.contains(&"vla".to_string()),
            "should match 'vla' as acronym of Vision-Language-Action; got: {tags:?}"
        );
        assert!(tags.contains(&"robotics".to_string()));
    }

    #[test]
    fn hyphenated_tag_segment_match() {
        let taxonomy = taxonomy_from(&["vq-vae", "diffusion"]);
        let tags = suggest_tags(
            "Discrete Latent Actions",
            Some("Uses VQ-VAE for action quantization"),
            &taxonomy,
        );
        assert!(tags.contains(&"vq-vae".to_string()));
    }

    #[test]
    fn literal_acronym_in_text() {
        let taxonomy = taxonomy_from(&["vla", "rl"]);
        let tags = suggest_tags("A New VLA Architecture", None, &taxonomy);
        assert!(tags.contains(&"vla".to_string()));
    }
}
