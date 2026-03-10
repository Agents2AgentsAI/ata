use serde::Deserialize;
use serde::Serialize;

/// A registered coordination session.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CoordinationSession {
    pub session_id: String,
    pub repo_path: String,
    pub branch: Option<String>,
    pub description: Option<String>,
    pub started_at: i64,
    pub last_heartbeat: i64,
}

/// A message posted to the coordination channel.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CoordinationMessage {
    pub id: i64,
    pub session_id: String,
    pub repo_path: String,
    pub message: String,
    pub message_type: String,
    pub created_at: i64,
}

/// Compute a project ID by hashing the normalized `git remote get-url origin`.
/// Returns `None` if the repo has no remote (local-only coordination).
#[cfg(feature = "relay")]
pub async fn compute_project_id(cwd: &std::path::Path) -> Option<String> {
    use sha2::Digest;
    use sha2::Sha256;

    let output = tokio::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(cwd)
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())?;

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        return None;
    }

    // Normalize: strip trailing .git and trailing slashes.
    let normalized = url
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/');

    let hash = Sha256::digest(normalized.as_bytes());
    Some(format!("{hash:x}"))
}

/// Derive a stable, human-readable name from a session ID.
///
/// Uses 3 bytes from the UUID to index into adjective × color × animal word
/// lists (64 × 64 × 64 = 262,144 combinations), producing names like
/// "swift-amber-falcon". The name is deterministic: the same session_id
/// always yields the same name.
pub fn agent_name(session_id: &str) -> String {
    const ADJECTIVES: [&str; 64] = [
        "swift", "bright", "calm", "bold", "keen", "warm", "cool", "sharp", "brave", "wise",
        "fair", "glad", "kind", "neat", "prime", "true", "vivid", "rapid", "steady", "lucid",
        "agile", "crisp", "fresh", "noble", "quiet", "solid", "vital", "eager", "firm", "loyal",
        "ready", "alert", "deft", "grand", "sleek", "stout", "dense", "fleet", "stark", "plush",
        "brisk", "daring", "clever", "gentle", "lively", "mighty", "nimble", "polite", "rugged",
        "serene", "tender", "witty", "zesty", "humble", "jovial", "placid", "supple", "terse",
        "astute", "candid", "dapper", "earnest", "fierce", "gutsy",
    ];
    const COLORS: [&str; 64] = [
        "amber", "azure", "coral", "ivory", "slate", "cedar", "flint", "jade", "onyx", "pearl",
        "ruby", "sage", "teal", "brass", "dusk", "ember", "frost", "gold", "hazel", "indigo",
        "khaki", "lilac", "maple", "navy", "opal", "plum", "quartz", "rose", "steel", "topaz",
        "umber", "violet", "wheat", "zinc", "aqua", "blush", "cocoa", "denim", "ebony", "fawn",
        "grain", "honey", "iris", "jet", "kelp", "lava", "mint", "nutmeg", "olive", "peach",
        "rain", "sand", "tangy", "ultra", "velvet", "walnut", "xenon", "yarrow", "zephyr", "ashen",
        "birch", "chalk", "dawn", "flax",
    ];
    const ANIMALS: [&str; 64] = [
        "falcon", "otter", "wolf", "hawk", "lynx", "crane", "puma", "wren", "fox", "bear", "heron",
        "tiger", "raven", "eagle", "viper", "bison", "finch", "gecko", "horse", "ibis", "koala",
        "lemur", "moose", "newt", "owl", "panda", "quail", "robin", "stork", "trout", "whale",
        "cobra", "dove", "elk", "frog", "gull", "hare", "jackal", "kite", "lark", "mink",
        "narwhal", "osprey", "parrot", "ray", "shark", "tern", "urchin", "vulture", "wombat",
        "yak", "zebra", "badger", "coyote", "drake", "ermine", "ferret", "grouse", "hyena",
        "iguana", "jaguar", "mantis", "python", "salmon",
    ];

    // Parse UUID hex bytes.
    let bytes: Vec<u8> = session_id
        .replace('-', "")
        .as_bytes()
        .chunks(2)
        .filter_map(|c| {
            std::str::from_utf8(c)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
        })
        .collect();

    let adj_idx = bytes.first().copied().unwrap_or(0) as usize % ADJECTIVES.len();
    let color_idx = bytes.get(1).copied().unwrap_or(0) as usize % COLORS.len();
    let animal_idx = bytes.get(2).copied().unwrap_or(0) as usize % ANIMALS.len();
    // 2 more bytes → 0..65535 numeric suffix for ~17 billion total combinations.
    let suffix = (bytes.get(3).copied().unwrap_or(0) as u16) << 8
        | bytes.get(4).copied().unwrap_or(0) as u16;
    format!(
        "{}-{}-{}-{}",
        ADJECTIVES[adj_idx], COLORS[color_idx], ANIMALS[animal_idx], suffix
    )
}
