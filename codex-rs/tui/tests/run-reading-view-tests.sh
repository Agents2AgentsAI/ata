#!/bin/bash
set -e

cd "$(dirname "$0")/../../.."
cd codex-rs

echo "=== Alignment & Karaoke Unit Tests (tests 1-8) ==="
cargo test -p codex-tui --features voice-input --lib -- alignment_single_word alignment_multiple_words alignment_cross_chunk_merge alignment_leading_whitespace_flushes alignment_empty_chunk alignment_all_whitespace alignment_punctuation_attached alignment_paragraph_break

echo ""
echo "=== find_active_word Unit Tests (tests 9-14) ==="
cargo test -p codex-tui --features voice-input --lib -- find_word_exact_start find_word_mid_word find_word_between_words find_word_before_first find_word_after_last find_word_empty_timeline

echo ""
echo "=== Render Highlight Unit Tests (tests 15-19) ==="
cargo test -p codex-tui --features voice-input --lib -- "document_reader::render::tests::highlight_"

echo ""
echo "=== Voice Reading Progress Unit Tests (tests 20-23) ==="
cargo test -p codex-tui --features voice-input --lib -- voice_progress_

echo ""
echo "=== Karaoke Integration Tests (tests 24-27 + extras) ==="
cargo test -p codex-tui --test karaoke_integration

echo ""
echo "=== E2E Reading View Tests (tests 28-46) ==="
cargo test -p codex-tui --features voice-input --lib -- "document_reader::tests::section_nav_n_p" "document_reader::tests::scroll_j_k" "document_reader::tests::half_page_scroll" "document_reader::tests::home_end_navigation_extended" "document_reader::tests::cursor_h_l_w_b" "document_reader::tests::visual_selection_v" "document_reader::tests::fold_toggle_f" "document_reader::tests::search_mode" "document_reader::tests::toc_overlay_toggle" "document_reader::tests::help_overlay_toggle" "document_reader::tests::q_twice_exits" "document_reader::tests::tab_opens_composer" "document_reader::tests::enter_submits_follow_up" "document_reader::tests::update_section_while_viewing" "document_reader::tests::append_section_increases_count" "document_reader::tests::rapid_n_presses" "document_reader::tests::resize_no_panic" "document_reader::tests::section_update_during_fold" "document_reader::tests::exit_during_voice_state"

echo ""
echo "=== Composer Cursor Tests ==="
cargo test -p codex-tui --lib -- "document_reader::tests::composer_cursor_wraps_to_next_line"

echo ""
echo "=== TTS E2E Tests (tests 47-49, require ELEVENLABS_API_KEY) ==="
if [ -n "$ELEVENLABS_API_KEY" ]; then
    cargo test -p codex-tui --test tts_e2e -- --ignored
else
    echo "Skipped (set ELEVENLABS_API_KEY to run)"
    cargo test -p codex-tui --test tts_e2e -- --list 2>&1 | grep ": test"
fi

echo ""
echo "=== Pause / Resume State Machine Tests ==="
cargo test -p codex-tui --features voice-input --lib -- tts_chunk_during_pause_does_not_transition_phase tts_chunk_transitions_to_speaking_when_not_paused tts_chunk_noop_when_already_speaking finalize_blocked_when_audio_paused finalize_allowed_when_audio_not_paused should_finalize_on_tick_blocked_when_paused should_finalize_on_tick_when_data_complete_and_drained resume_with_drained_buffer_does_not_enter_speaking resume_with_buffered_audio_succeeds pause_blocks_finalization_then_resume_without_audio_allows_it tts_data_incomplete_prevents_finalization_even_when_buffer_empty resume_after_finalization_does_not_resume

echo ""
echo "=== S-Key Pause/Resume + Space PTT E2E Tests ==="
cargo test -p codex-tui --features voice-input --lib -- s_key_sends_pause_when_voice_active s_key_sends_resume_when_paused space_in_reading_view_triggers_ptt_not_pause

echo ""
echo "=== All reading view & karaoke tests complete ==="
