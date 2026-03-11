//! Fixture recorder for TTS alignment data.
//!
//! This is an `#[ignore]` test that connects to ElevenLabs, sends known texts,
//! and records the alignment responses to JSON fixture files for use in karaoke
//! pipeline integration tests.
//!
//! Run manually:
//! ```sh
//! ELEVENLABS_API_KEY=sk-... cargo test -p codex-elevenlabs --test record_fixtures -- --ignored
//! ```

use codex_elevenlabs::ElevenLabsConfig;
use codex_elevenlabs::TtsAlignment;
use codex_elevenlabs::tts::TtsStream;
use serde::Serialize;

// ─── Fixture output types ────────────────────────────────────────────────────

#[derive(Serialize)]
struct FixtureFile {
    input_text: String,
    chunks: Vec<FixtureChunk>,
}

#[derive(Serialize)]
struct FixtureChunk {
    pcm_samples: usize,
    alignment: FixtureAlignment,
}

#[derive(Serialize)]
struct FixtureAlignment {
    chars: Vec<String>,
    char_start_times_ms: Vec<u64>,
    char_durations_ms: Vec<u64>,
}

impl From<&TtsAlignment> for FixtureAlignment {
    fn from(a: &TtsAlignment) -> Self {
        Self {
            chars: a.chars.clone(),
            char_start_times_ms: a.char_start_times_ms.clone(),
            char_durations_ms: a.char_durations_ms.clone(),
        }
    }
}

// ─── Test texts ──────────────────────────────────────────────────────────────

const TEXTS: &[(&str, &str)] = &[
    ("short", "Hello, world."),
    ("medium", "The quick brown fox jumps over the lazy dog."),
    (
        "long",
        "In the beginning, there was only darkness. Then a spark of light \
         emerged, growing brighter with each passing moment. The universe \
         began to take shape.",
    ),
];

// ─── Fixture recorder ────────────────────────────────────────────────────────

/// Record TTS fixtures from ElevenLabs.
///
/// Requires `ELEVENLABS_API_KEY` environment variable. Saves fixture files to
/// `codex-rs/tui/tests/fixtures/tts_{name}.json`.
#[tokio::test]
#[ignore]
async fn record_tts_fixtures() {
    let api_key = std::env::var("ELEVENLABS_API_KEY")
        .unwrap_or_else(|_| panic!("ELEVENLABS_API_KEY not set"));
    assert!(!api_key.is_empty(), "ELEVENLABS_API_KEY is empty");

    // Output directory: codex-rs/tui/tests/fixtures/
    let fixtures_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| panic!("no parent of CARGO_MANIFEST_DIR"))
        .join("tui")
        .join("tests")
        .join("fixtures");
    std::fs::create_dir_all(&fixtures_dir)
        .unwrap_or_else(|e| panic!("failed to create fixtures dir: {e}"));

    for (name, text) in TEXTS {
        eprintln!("Recording fixture: {name} ({text:?})");

        let config = ElevenLabsConfig::new(api_key.clone());
        let mut stream = TtsStream::connect(&config)
            .await
            .unwrap_or_else(|e| panic!("failed to connect for '{name}': {e}"));

        stream
            .send_text(text)
            .await
            .unwrap_or_else(|e| panic!("failed to send text for '{name}': {e}"));
        stream
            .flush()
            .await
            .unwrap_or_else(|e| panic!("failed to flush for '{name}': {e}"));
        stream.send_eos().await;

        let mut chunks: Vec<FixtureChunk> = Vec::new();
        while let Some(chunk) = stream.recv_audio().await {
            let alignment = match chunk.alignment.as_ref() {
                Some(a) => FixtureAlignment::from(a),
                None => {
                    eprintln!("  chunk {} has no alignment, skipping", chunks.len() + 1);
                    continue;
                }
            };
            eprintln!(
                "  chunk {}: {} PCM samples, {} alignment chars",
                chunks.len() + 1,
                chunk.pcm.len(),
                alignment.chars.len(),
            );
            chunks.push(FixtureChunk {
                pcm_samples: chunk.pcm.len(),
                alignment,
            });
        }

        assert!(
            !chunks.is_empty(),
            "expected at least one chunk for '{name}'"
        );

        let fixture = FixtureFile {
            input_text: text.to_string(),
            chunks,
        };

        let path = fixtures_dir.join(format!("tts_{name}.json"));
        let json = serde_json::to_string_pretty(&fixture)
            .unwrap_or_else(|e| panic!("failed to serialize fixture '{name}': {e}"));
        std::fs::write(&path, json)
            .unwrap_or_else(|e| panic!("failed to write {}: {e}", path.display()));

        eprintln!("  Saved: {}", path.display());
    }
}
