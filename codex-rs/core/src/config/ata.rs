//! ATA-private config types (Supabase auth, relay routing).
//!
//! Surfaced through `codex_core::config::types::*` so the TUI and any other
//! consumer can import them via the standard config types path.

use serde::Deserialize;
use serde::Serialize;

/// ATA Supabase project URL.
pub const DEFAULT_ATA_SUPABASE_URL: &str = "https://natbqqfawsmcoeutsogu.supabase.co";

/// ATA Supabase publishable (anon) key.
pub const DEFAULT_ATA_SUPABASE_ANON_KEY: &str = "sb_publishable_MopvwXLh_k866kZvplSGGQ_IBircspG";

/// Configuration for ATA account features (Supabase-backed auth and relay).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AtaAccountConfig {
    #[serde(default = "default_ata_supabase_url")]
    pub supabase_url: String,
    #[serde(default = "default_ata_supabase_anon_key")]
    pub supabase_anon_key: String,
    #[serde(default)]
    pub relay_mode: RelayMode,
}

impl Default for AtaAccountConfig {
    fn default() -> Self {
        Self {
            supabase_url: default_ata_supabase_url(),
            supabase_anon_key: default_ata_supabase_anon_key(),
            relay_mode: RelayMode::default(),
        }
    }
}

fn default_ata_supabase_url() -> String {
    DEFAULT_ATA_SUPABASE_URL.to_string()
}

fn default_ata_supabase_anon_key() -> String {
    DEFAULT_ATA_SUPABASE_ANON_KEY.to_string()
}

/// Relay connection mode for ATA accounts.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum RelayMode {
    #[default]
    Cloud,
    Local,
    Custom {
        relay_url: String,
    },
}
