# TTS/Karaoke Sync Test Harness

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A manually-triggered test that runs markdown through the exact production TTS/karaoke pipeline, sends audio to ElevenLabs STT, and produces a detailed report showing what was spoken, what was heard back, what the browser highlights, and what the TUI highlights — so an agent can read the report and judge correctness.

**Architecture:** A Rust integration test (`tts_sync_report.rs`) that exercises the real text preparation functions, real ElevenLabs TTS+STT, and simulates browser/TUI karaoke rendering. Outputs a markdown report to `/tmp/tts-sync-report.md`. A Playwright test (`tts_browser_sync.ts`) loads the actual `LivingReadingView.html` and captures real DOM highlight state during karaoke playback.

**Tech Stack:** Rust (codex-tui, codex-elevenlabs), ElevenLabs TTS+STT API, Playwright (browser capture)

---

### Task 1: Export text preparation functions for integration tests

The test needs access to production functions that are currently `pub(crate)`. Add `#[doc(hidden)]` re-exports gated on `voice-input` feature, same pattern as the existing `AlignmentEntry`/`build_alignment_entries`/`find_active_word` exports.

**Files:**
- Modify: `codex-rs/tui/src/lib.rs:71-80`
- Modify: `codex-rs/tui/src/chatwidget/voice_mode.rs` (make `SentenceBuffer` visible)
- Modify: `codex-rs/tui/src/chatwidget_document_reader.rs` (make `browser_read_aloud_markup` visible)

- [ ] **Step 1: Add exports to lib.rs**

Add after the existing `find_active_word` export (line 80):

```rust
#[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
#[doc(hidden)]
pub use chatwidget::voice_mode::parse_equation_markers;
#[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
#[doc(hidden)]
pub use chatwidget::voice_mode::clean_for_tts_preserving_equation_markers;
#[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
#[doc(hidden)]
pub use chatwidget::voice_mode::SentenceBuffer;
#[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
#[doc(hidden)]
pub use chatwidget_document_reader::browser_read_aloud_markup;
```

- [ ] **Step 2: Adjust visibility of target items**

In `voice_mode.rs`: `SentenceBuffer` is currently `pub(crate)` — change to `pub` (the `#[doc(hidden)]` re-export handles API surface). Similarly ensure `parse_equation_markers` and `clean_for_tts_preserving_equation_markers` are `pub`.

In `chatwidget_document_reader.rs`: `browser_read_aloud_markup` is a private free function — change to `pub(crate)`.

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p codex-tui --features voice-input`

- [ ] **Step 4: Commit**

```
feat: export TTS text preparation functions for integration tests
```

---

### Task 2: Add PCM-to-WAV helper in test support

The STT API needs WAV bytes. Add a small helper that wraps raw i16 PCM (24kHz mono) into a WAV file.

**Files:**
- Modify: `codex-rs/tui/tests/support/recorded_tts.rs`

- [ ] **Step 1: Add `pcm_to_wav` function**

```rust
/// Encode raw PCM i16 samples (24kHz mono) into a WAV byte buffer.
pub fn pcm_to_wav(pcm: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let file_len = 36 + data_len;
    let mut buf = Vec::with_capacity(file_len as usize + 8);
    // RIFF header
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&file_len.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    // fmt chunk
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    buf.extend_from_slice(&1u16.to_le_bytes());  // PCM format
    buf.extend_from_slice(&1u16.to_le_bytes());  // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes());  // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    // data chunk
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for &sample in pcm {
        buf.extend_from_slice(&sample.to_le_bytes());
    }
    buf
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo test -p codex-tui --test tts_e2e -- --list`

- [ ] **Step 3: Commit**

```
feat: add pcm_to_wav helper for TTS sync tests
```

---

### Task 3: Create the sync report test — text preparation + TTS + STT

The main test file. Each test case runs markdown through the production pipeline, sends to TTS, gets STT back, and writes a report section.

**Files:**
- Create: `codex-rs/tui/tests/tts_sync_report.rs`

- [ ] **Step 1: Create test file with shared infrastructure**

```rust
//! TTS/Karaoke sync report generator.
//!
//! Runs test cases through the exact production text preparation pipeline,
//! sends to ElevenLabs TTS, transcribes back via STT, simulates browser/TUI
//! karaoke rendering, and writes a detailed report to /tmp/tts-sync-report.md.
//!
//! Run manually:
//! ```sh
//! ELEVENLABS_API_KEY=sk-... cargo test -p codex-tui --test tts_sync_report -- --ignored --nocapture
//! ```

#![cfg(not(target_os = "linux"))]
#![allow(clippy::expect_used)]

mod support;

use std::fmt::Write as FmtWrite;
use std::io::Write;

use codex_elevenlabs::ElevenLabsConfig;
use codex_elevenlabs::tts::TtsStream;
use codex_tui::{
    AlignmentEntry, SentenceBuffer, build_alignment_entries, find_active_word,
    parse_equation_markers, clean_for_tts_preserving_equation_markers,
    browser_read_aloud_markup,
};
use support::recorded_tts::pcm_to_wav;

struct TestCase {
    name: &'static str,
    markdown: &'static str,
}

struct Report {
    sections: Vec<String>,
}

impl Report {
    fn new() -> Self { Self { sections: Vec::new() } }

    fn add(&mut self, section: String) { self.sections.push(section); }

    fn write_to_file(&self, path: &str) {
        let mut f = std::fs::File::create(path).expect("create report file");
        writeln!(f, "# TTS/Karaoke Sync Report\n").unwrap();
        writeln!(f, "Generated: {}\n", chrono_now()).unwrap();
        for s in &self.sections {
            writeln!(f, "{s}\n").unwrap();
        }
    }
}

fn chrono_now() -> String {
    // Simple timestamp without pulling in chrono crate
    format!("{:?}", std::time::SystemTime::now())
}

fn test_cases() -> Vec<TestCase> {
    vec![
        TestCase {
            name: "plain_short",
            markdown: "Hello, world.",
        },
        TestCase {
            name: "plain_multi_sentence",
            markdown: "First sentence here. Second sentence follows.",
        },
        TestCase {
            name: "inline_equation",
            markdown: "The result is <eq latex=\"x^2\">x squared</eq> plus one.",
        },
        TestCase {
            name: "multiple_equations",
            markdown: "Given <eq latex=\"I = \\{M\\} \\cup W\">I equals M union W</eq>, \
                       the state is <eq latex=\"s\">s</eq>.",
        },
        TestCase {
            name: "block_equation",
            markdown: "Consider:\n\n<eq display=\"block\" latex=\"E = mc^2\">\
                       E equals m c squared</eq>\n\nThis is famous.",
        },
        TestCase {
            name: "markdown_list",
            markdown: "- First item\n- Second item\n- Third item",
        },
        TestCase {
            name: "inline_code",
            markdown: "The `main()` function runs first.",
        },
        TestCase {
            name: "bold_italic",
            markdown: "**Bold text** and *italic text* here.",
        },
        TestCase {
            name: "consecutive_equations",
            markdown: "Values <eq latex=\"A\">A</eq> and <eq latex=\"B\">B</eq> \
                       and <eq latex=\"C\">C</eq> are given.",
        },
        TestCase {
            name: "paper_excerpt",
            markdown: "The paper models workflow orchestration as a partially observable \
                       stochastic game. The participants are the manager plus a set of \
                       workers, written as <eq latex=\"I = \\{M\\} \\cup W\">I equals \
                       the manager together with the worker set</eq>. The environment \
                       state is <eq latex=\"s = \\langle G, W, C, X, U \\rangle\">state \
                       equals task graph, workers, communication history, artifacts, and \
                       stakeholder preferences</eq>.",
        },
    ]
}
```

- [ ] **Step 2: Add the per-test-case pipeline function**

This is the core. For each test case, it runs the full pipeline and returns a report section.

```rust
async fn run_test_case(tc: &TestCase, config: &ElevenLabsConfig) -> String {
    let mut out = String::new();
    writeln!(out, "## {}\n", tc.name).unwrap();
    writeln!(out, "**Input markdown:**\n```\n{}\n```\n", tc.markdown).unwrap();

    // ── Phase 1: Text preparation (exact production functions) ──
    let markup = browser_read_aloud_markup(tc.markdown);
    writeln!(out, "**After browser_read_aloud_markup:**\n```\n{markup}\n```\n").unwrap();

    let (tts_text, eq_spans) = parse_equation_markers(&markup);
    writeln!(out, "**Sent to TTS (markers stripped):**\n```\n{tts_text}\n```\n").unwrap();
    writeln!(out, "**Equation spans:** {eq_spans:?}\n").unwrap();

    let mut sb = SentenceBuffer::new();
    let mut sentences = sb.push(&tts_text);
    if let Some(r) = sb.flush() { sentences.push(r); }
    writeln!(out, "**Sentences:** {sentences:?}\n").unwrap();

    // ── Phase 2: TTS ──
    let mut stream = TtsStream::connect(config).await.expect("TTS connect");
    for sentence in &sentences {
        stream.send_text(sentence).await.expect("TTS send");
        stream.flush().await.expect("TTS flush");
    }
    stream.send_eos().await;

    let mut chunks = Vec::new();
    while let Some(chunk) = stream.recv_audio().await {
        chunks.push(chunk);
    }

    // Build timeline
    let mut timeline: Vec<AlignmentEntry> = Vec::new();
    let mut pending_word: Option<AlignmentEntry> = None;
    let mut all_pcm: Vec<i16> = Vec::new();
    for chunk in &chunks {
        if let Some(ref align) = chunk.alignment {
            build_alignment_entries(align, 0, &mut timeline, &mut pending_word);
        }
        all_pcm.extend_from_slice(&chunk.pcm);
    }
    if let Some(pw) = pending_word.take() { timeline.push(pw); }

    writeln!(out, "**Timeline ({} words, {} chunks, {}ms audio):**\n",
        timeline.len(), chunks.len(), all_pcm.len() as u64 * 1000 / 24000).unwrap();
    writeln!(out, "| # | Word | Start (ms) | Duration (ms) | Eq? |").unwrap();
    writeln!(out, "|---|------|-----------|--------------|-----|").unwrap();
    for (i, entry) in timeline.iter().enumerate() {
        let eq_label = eq_spans.iter()
            .find(|(_, s, e)| i >= *s && i < *e)
            .map(|(idx, _, _)| format!("EQ:{idx}"))
            .unwrap_or_default();
        writeln!(out, "| {i} | {} | {} | {} | {eq_label} |",
            entry.word, entry.start_ms, entry.duration_ms).unwrap();
    }
    writeln!(out).unwrap();

    // ── Phase 3: STT ──
    let wav = pcm_to_wav(&all_pcm, 24000);
    match codex_elevenlabs::stt::transcribe(config, wav).await {
        Ok(transcript) => {
            writeln!(out, "**STT heard back:**\n```\n{transcript}\n```\n").unwrap();
        }
        Err(e) => {
            writeln!(out, "**STT ERROR:** {e}\n").unwrap();
        }
    }

    // ── Phase 4: Browser karaoke mapping ──
    let spoken_total = tts_text.split_whitespace().count();
    let hidden_eq: usize = eq_spans.iter().map(|(_, s, e)| e - s).sum();
    let visible_total = spoken_total - hidden_eq;
    writeln!(out, "**Browser word mapping** (spoken={spoken_total}, hidden_eq={hidden_eq}, visible={visible_total}):\n").unwrap();
    writeln!(out, "| Spoken idx | Visible idx | Word | Eq highlight |").unwrap();
    writeln!(out, "|-----------|------------|------|-------------|").unwrap();
    let tts_words: Vec<&str> = tts_text.split_whitespace().collect();
    for spoken_idx in 0..spoken_total {
        let mut hidden = 0usize;
        for &(_, start, end) in &eq_spans {
            if start >= spoken_idx { break; }
            let overlap_end = spoken_idx.min(end);
            if overlap_end > start { hidden += overlap_end - start; }
        }
        let visible_idx = spoken_idx.saturating_sub(hidden);
        let active_eq = eq_spans.iter()
            .find(|(_, s, e)| spoken_idx >= *s && spoken_idx < *e)
            .map(|(idx, _, _)| format!("glow EQ:{idx}"));
        let word = tts_words.get(spoken_idx).unwrap_or(&"???");
        writeln!(out, "| {spoken_idx} | {visible_idx} | {word} | {} |",
            active_eq.as_deref().unwrap_or("")).unwrap();
    }
    writeln!(out).unwrap();

    // ── Phase 5: TUI karaoke mapping ──
    writeln!(out, "**TUI highlight walk** (10ms steps, first 20 transitions):\n").unwrap();
    writeln!(out, "| Time (ms) | Word idx | Word |").unwrap();
    writeln!(out, "|----------|---------|------|").unwrap();
    let total_ms = timeline.last().map(|e| e.start_ms + e.duration_ms).unwrap_or(0);
    let mut last_idx: Option<usize> = None;
    let mut transitions = 0;
    let mut t = 0u64;
    while t <= total_ms && transitions < 20 {
        let idx = find_active_word(&timeline, t);
        if idx != last_idx {
            let word = idx.and_then(|i| timeline.get(i)).map(|e| e.word.as_str()).unwrap_or("-");
            writeln!(out, "| {t} | {} | {word} |", idx.map(|i| i.to_string()).unwrap_or("-".into())).unwrap();
            last_idx = idx;
            transitions += 1;
        }
        t += 10;
    }
    writeln!(out).unwrap();

    // ── Phase 6: Interrupt/restart ──
    writeln!(out, "**Interrupt test:** Restart TTS, verify fresh timeline starts near 0ms\n").unwrap();
    let mut stream2 = TtsStream::connect(config).await.expect("TTS reconnect");
    stream2.send_text(&tts_text).await.expect("TTS send2");
    stream2.flush().await.expect("TTS flush2");
    stream2.send_eos().await;
    let mut timeline2: Vec<AlignmentEntry> = Vec::new();
    let mut pending2: Option<AlignmentEntry> = None;
    while let Some(chunk) = stream2.recv_audio().await {
        if let Some(ref align) = chunk.alignment {
            build_alignment_entries(align, 0, &mut timeline2, &mut pending2);
        }
    }
    if let Some(pw) = pending2.take() { timeline2.push(pw); }
    let restart_ok = timeline2.first().map(|e| e.start_ms < 500).unwrap_or(false);
    let restart_count_ok = timeline2.len() == tts_text.split_whitespace().count();
    writeln!(out, "- Restart first word at: {}ms (expect <500ms) {}",
        timeline2.first().map(|e| e.start_ms).unwrap_or(9999),
        if restart_ok { "OK" } else { "FAIL" }).unwrap();
    writeln!(out, "- Restart word count: {} (expect {}) {}",
        timeline2.len(), tts_text.split_whitespace().count(),
        if restart_count_ok { "OK" } else { "FAIL" }).unwrap();
    writeln!(out, "\n---\n").unwrap();

    out
}
```

- [ ] **Step 3: Add the main test that runs all cases and writes the report**

```rust
#[tokio::test]
#[ignore]
async fn generate_tts_sync_report() {
    let api_key = std::env::var("ELEVENLABS_API_KEY")
        .expect("ELEVENLABS_API_KEY must be set");
    let config = ElevenLabsConfig::new(api_key);
    let cases = test_cases();
    let mut report = Report::new();

    for tc in &cases {
        eprintln!("Running: {} ...", tc.name);
        let section = run_test_case(tc, &config).await;
        report.add(section);
    }

    let path = "/tmp/tts-sync-report.md";
    report.write_to_file(path);
    eprintln!("\nReport written to {path}");
    eprintln!("Review with: cat {path}");
}
```

- [ ] **Step 4: Verify compilation**

Run: `cargo test -p codex-tui --test tts_sync_report -- --list`

- [ ] **Step 5: Commit**

```
feat: add TTS/karaoke sync report test
```

---

### Task 4: Add `just test-tts-sync` command

**Files:**
- Modify: `codex-rs/justfile`

- [ ] **Step 1: Add command after existing `test-tts-live`**

```just
test-tts-sync:
    cargo test -p codex-tui --features voice-input --test tts_sync_report -- --ignored --nocapture
    @echo "Report: /tmp/tts-sync-report.md"
```

- [ ] **Step 2: Commit**

```
feat: add just test-tts-sync command
```

---

### Task 5: Browser capture via Playwright

Uses Playwright to load the actual `LivingReadingView.html`, inject test content, trigger karaoke, and capture which DOM elements are highlighted at each word transition.

**Files:**
- Create: `codex-rs/tui/tests/browser_karaoke_capture.ts`
- Modify: `codex-rs/justfile` (add `test-tts-browser` command)

- [ ] **Step 1: Create the Playwright test script**

This script:
1. Starts the reading-view-server on a random port
2. Opens the page in Playwright
3. Injects a test section with equations via the WebSocket `updateDocumentSection` message
4. Listens for `karaokeWord` messages on the WebSocket
5. At each word transition, queries the DOM for which `.kw` span has the `revealed` class as the latest, and which `.katex-eq-active` elements exist
6. Writes a report to `/tmp/tts-browser-report.md`

The exact implementation depends on the reading-view-server's WebSocket protocol. The test should use `@playwright/test` and connect to the same WebSocket the browser uses.

Key captures at each `karaokeWord` message:
```typescript
const revealedWords = await page.$$eval('.kw.revealed', els =>
    els.map(el => ({ index: el.dataset.wi, text: el.textContent }))
);
const activeEqs = await page.$$eval('.katex-eq-active', els =>
    els.map(el => el.textContent.trim())
);
```

- [ ] **Step 2: Add just command**

```just
test-tts-browser:
    npx playwright test codex-rs/tui/tests/browser_karaoke_capture.ts
    @echo "Report: /tmp/tts-browser-report.md"
```

- [ ] **Step 3: Commit**

```
feat: add Playwright browser karaoke capture test
```

---

### Task 6: TUI capture via vt100 test backend

Uses the existing vt100 test backend to render the document reader with karaoke active, stepping through the timeline and capturing the terminal output at each word transition.

**Files:**
- Create: `codex-rs/tui/tests/tui_karaoke_capture.rs`

- [ ] **Step 1: Create TUI capture test**

This test:
1. Creates a vt100 backend widget with a document reader showing a test section
2. Feeds alignment data and simulates `VoiceModeHighlightTick` events at each word's timestamp
3. After each tick, renders the widget and captures the terminal buffer
4. Extracts the highlighted (styled) text from the buffer
5. Writes a report showing what text was highlighted at each timestamp

The report format:
```
## TUI Karaoke Capture: inline_equation

| Time (ms) | Highlighted text on screen |
|----------|--------------------------|
| 0        | **The** result is x squared plus one. |
| 140      | The **result** is x squared plus one. |
| 360      | The result **is** x squared plus one. |
```

- [ ] **Step 2: Commit**

```
feat: add TUI karaoke capture test
```

---

## Execution Order

Tasks 1-4 are the core — they produce the sync report that agents read.
Task 5 (browser) and Task 6 (TUI) add rendered capture and can be done after the core is validated.

The self-healing loop is: `just test-tts-sync` → agent reads `/tmp/tts-sync-report.md` → fixes code → repeats.
