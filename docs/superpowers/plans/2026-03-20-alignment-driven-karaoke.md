# Alignment-Driven Karaoke

> **For agentic workers:** This replaces the current dual-word-counter architecture with a single source of truth.

**Goal:** Eliminate karaoke word count drift by using the TTS alignment timeline as the sole word list. The browser wraps words based on what TTS actually spoke, not independent heuristics.

**Why:** Currently two systems independently decide "what is a word":
- TTS side: `clean_for_tts` → ElevenLabs → alignment timeline (N words)
- Browser side: `_walkAndWrap` → `.kw` spans (M words)

When N ≠ M (e.g. standalone `=` is spoken but skipped in DOM), karaoke drifts. This has produced a growing pile of heuristic exemptions (`_isLatexLikeToken`, `isMathLikeToken`, standalone symbol rules, multiplier patterns). Each new edge case needs a fix in both systems.

**New architecture:** The alignment IS the word list. The browser matches spoken words to DOM text instead of independently counting.

---

## How it works

### Cached sections (full alignment available)

1. User clicks read-aloud → Rust checks cache → cache hit
2. Rust sends `startKaraoke` with the alignment word list:
   ```json
   { "type": "startKaraoke", "sectionIndex": 2,
     "words": ["My", "read", "is", ..., "=", "better", ...],
     "totalWords": 92 }
   ```
3. Browser receives the word list
4. New `_wrapWordsFromAlignment(words)` walks the DOM, finds each word in order, wraps as `.kw[data-wi=N]`
5. Karaoke proceeds as before: `karaokeWord` messages highlight `.kw` spans

### Live streaming (alignment arrives chunk by chunk)

1. User triggers read-aloud → Rust starts TTS, sends `startKaraoke` with empty/partial word list
2. As alignment chunks arrive in `on_voice_tts_audio_chunk`:
   - `build_alignment_entries` produces new words
   - Rust sends incremental word notifications:
     ```json
     { "type": "karaokeWordsAdded", "startIndex": 42, "words": ["=", "better", "reasoning."] }
     ```
   - Browser wraps these new words in the DOM as they arrive
3. `karaokeWord` messages highlight words as before — but now the indices are guaranteed to match because the browser used the alignment's word list

### The new `_wrapWordsFromAlignment`

Instead of splitting text on whitespace and applying heuristic filters, this function:

1. Takes an array of word strings from the alignment
2. Maintains a DOM cursor (current text node + offset)
3. For each word: advances the cursor through the DOM, skipping KaTeX blocks and `<pre>` tags
4. When it finds matching text, wraps it in a `.kw` span
5. Any text between wrapped words (punctuation, symbols) becomes `.kw-decor` automatically

This means:
- No `_isLatexLikeToken` — if the word isn't in the alignment, it doesn't get wrapped
- No `clean_for_tts` ↔ `_walkAndWrap` sync issues — one word list drives both
- Standalone `=`, `±`, `17×` — if TTS speaks them, they appear in the alignment, they get wrapped
- KaTeX blocks — TTS speaks the equation paraphrase, not the LaTeX; the DOM cursor skips KaTeX elements and the paraphrase words aren't in the visible DOM (handled by equation word spans as before)

---

## Changes needed

### Rust (`codex-rs/`)

**`tui/src/chatwidget/voice_mode.rs`:**
- In `ensure_tts_playback_started`: include alignment word list in `startKaraoke` message
- In `on_voice_tts_audio_chunk`: send `karaokeWordsAdded` for newly discovered words
- Keep `equationWordSpans` and `visible_word_idx` logic (equations still need special handling since the spoken paraphrase doesn't appear in the DOM)

**`reading-view-server/src/assets/LivingReadingView.html`:**
- Replace `_walkAndWrap` with `_wrapWordsFromAlignment(words)`
- Add handler for `karaokeWordsAdded` message
- Modify `startKaraoke` to accept optional `words` array
- Remove `_isLatexLikeToken` function (no longer needed)
- Keep `.kw-decor` for non-word content between alignment words

### Swift (`ata-swift/`)

**`ATA/ATA/Extensions/RichTextView.swift`:**
- Same JS changes as Rust HTML (they share the same karaoke JS pattern)
- `wrapWordsForKaraoke()` becomes `wrapWordsFromAlignment(words)`
- Remove `_isLatexLikeToken`

**`ATA/ATA/UI/ReadingView/TeleprompterView.swift`:**
- Same JS changes

**`ATA/ATA/Voice/VoiceManager.swift`:**
- When starting karaoke, pass alignment word list to WebView via `evaluateJavaScript`
- For streaming: send incremental words as alignment builds

### What stays the same

- `AlignmentTimelineBuilder` / `build_alignment_entries` — still builds the timeline
- `equationWordSpans` — still needed for visible↔spoken mapping (equations are spoken but not in DOM)
- `visibleWordCount(forSpokenRevealedCount:)` — still needed
- `clean_for_tts` / `stripMarkdown` — still prepares text for TTS (but no longer needs to match browser word counting)
- `SentenceBuffer` — still splits text for TTS streaming

### What gets removed

- `_isLatexLikeToken` function in all three HTML/JS locations
- `_walkAndWrap` heuristic word splitting (replaced by alignment-driven wrapping)
- All standalone symbol exemptions (`=`, `+`, `±`, `17×` rules)
- The contract that `clean_for_tts` word count must match `_walkAndWrap` word count

---

## Edge cases

1. **Streaming latency**: First few words might arrive before the browser has them. Buffer a small number of `karaokeWord` messages until their corresponding `karaokeWordsAdded` arrives, then reveal them immediately.

2. **Cross-chunk words**: `build_alignment_entries` handles words split across chunks via `pending_word`. The `karaokeWordsAdded` message should only include fully resolved words (not pending partial words).

3. **Equation paraphrases**: The alignment includes spoken equation words (e.g. "x squared") that don't appear in the DOM (DOM has KaTeX). The equation word spans mechanism still handles this — those words advance the spoken index but not the visible index.

4. **DOM text matching**: The alignment word might be "reasoning." but the DOM text node contains `reasoning.` followed by `"` (curly quote from marked.js). The matcher should trim and compare ignoring punctuation differences. Or better: match by position (Nth word in the content) rather than by text equality.

---

## Testing

Extend `just test-tts-sync` to verify that the alignment word list matches the DOM `.kw` span text. Add a Phase 6 to the report:

```
## Phase 6 — Alignment-DOM word match
| alignment_idx | alignment_word | dom_kw_text | match |
|--------------|---------------|------------|-------|
| 0 | My | My | ✓ |
| 42 | = | = | ✓ |
```

If every alignment word has a corresponding `.kw` span with matching text, the systems are in sync.
