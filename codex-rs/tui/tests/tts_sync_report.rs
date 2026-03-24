//! TTS/karaoke synchronization report generator.
//!
//! Runs markdown test cases through the exact production TTS/karaoke pipeline,
//! sends audio to ElevenLabs TTS to get alignment data, then to STT to get a
//! transcript, simulates browser and TUI karaoke word mapping, and writes a
//! detailed markdown report to `/tmp/tts-sync-report.md`.
//!
//! All tests are `#[ignore]` by default — they require a valid
//! `ELEVENLABS_API_KEY` environment variable and make real API calls.
//!
//! Run manually:
//! ```sh
//! ELEVENLABS_API_KEY=sk-... cargo test -p codex-tui --features voice-input --test tts_sync_report -- --ignored --nocapture
//! ```

// codex-elevenlabs is only available on non-Linux platforms.
#![cfg(not(target_os = "linux"))]
#![allow(clippy::expect_used)]

mod support;

use codex_elevenlabs::ElevenLabsConfig;
use codex_elevenlabs::TtsChunk;
use codex_elevenlabs::tts::TtsStream;

use codex_tui::AlignmentEntry;
use codex_tui::SentenceBuffer;
use codex_tui::browser_read_aloud_markup;
use codex_tui::build_alignment_entries;
use codex_tui::clean_for_tts_preserving_equation_markers;
use codex_tui::find_active_word;
use codex_tui::parse_equation_markers;
use codex_tui::repair_timeline_monotonicity;
use codex_tui::strip_pause_markers;

use support::recorded_tts::pcm_to_wav;

use std::fmt::Write as FmtWrite;

// ---------------------------------------------------------------------------
// Test cases
// ---------------------------------------------------------------------------

struct TestCase {
    name: &'static str,
    markdown: &'static str,
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
            markdown: "Given <eq latex=\"I\">I equals M union W</eq>, the state is <eq latex=\"s\">s</eq>.",
        },
        TestCase {
            name: "block_equation",
            markdown: "Consider:\n\n<eq display=\"block\" latex=\"E = mc^2\">E equals m c squared</eq>\n\nThis is famous.",
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
            markdown: "Values <eq latex=\"A\">A</eq> and <eq latex=\"B\">B</eq> and <eq latex=\"C\">C</eq> are given.",
        },
        TestCase {
            name: "paper_excerpt",
            markdown: "We define the workflow orchestration problem as follows. \
                Given a set of agents <eq latex=\"I = \\{M\\} \\cup W\">I equals M union W</eq>, \
                where <eq latex=\"M\">M</eq> is the manager and <eq latex=\"W\">W</eq> is the \
                set of worker agents, and a state <eq latex=\"s \\in S\">s in S</eq>, the manager \
                selects an action <eq latex=\"a = \\pi_M(s)\">a equals pi M of s</eq>.",
        },
        TestCase {
            name: "real_section_state_equation",
            markdown: "The paper models the setting as a partially observable stochastic game. \
                In plain English, that means the manager acts in a world it cannot fully observe, \
                where outcomes are uncertain, and where other actors may have their own behavior \
                patterns or incentives. That is a good fit for real teamwork: the manager never \
                knows everything, workers are imperfect, and the environment changes as the \
                project unfolds.\n\n\
                The state is written as \
                <eq latex=\"s = \\langle G, W, C, X, U \\rangle\">state equals angle bracket G, W, C, X, U angle bracket</eq>. \
                Here, <eq latex=\"G\" speak=\"G\"/> is the workflow graph, \
                <eq latex=\"W\" speak=\"W\"/> is the worker pool, \
                <eq latex=\"C\" speak=\"C\"/> is the communication record, \
                <eq latex=\"X\" speak=\"X\"/> is the artifact store, \
                and <eq latex=\"U\" speak=\"U\"/> is the stakeholder preference vector over things \
                like speed, quality, cost, and compliance. That is the key abstraction in the paper: \
                the manager is not only scheduling tasks, it is managing tasks, people, artifacts, \
                and stakeholder tradeoffs together.\n\n\
                Its actions are intentionally broad. The manager can inspect tasks, decompose or \
                refine the graph, add dependencies, assign tasks, and message workers. That is \
                important because the benchmark is not testing a pure dispatcher. It is testing \
                whether a manager can **shape the workflow itself** as more information arrives.\n\n\
                The reward is also multi-objective. Success is not one scalar like \"final answer \
                correct.\" Instead, the system has to trade off completion, constraint adherence, \
                stakeholder management, and time. That formal choice is one of the strongest ideas \
                in the paper, because it reflects why orchestration is difficult in practice.",
        },
        TestCase {
            name: "stats_with_symbols",
            markdown: "The authors evaluate three GPT-5-based manager policies over the 20 \
                workflows and 5 random seeds each, with at most 100 manager actions per \
                episode. The baselines are Random, CoT, and Assign-All. The headline result \
                is that none of them jointly optimize the full problem. Average goal \
                achievement is 0.135 \u{00B1} 0.098 for Random, 0.313 \u{00B1} 0.187 for \
                CoT, and 0.502 \u{00B1} 0.209 for Assign-All. Constraint adherence is \
                0.432 \u{00B1} 0.095 for Random, 0.589 \u{00B1} 0.140 for CoT, and \
                0.475 \u{00B1} 0.080 for Assign-All. CoT completes more task nodes but is \
                slow: average runtime rises to 46.9 hours versus 2.7 hours for Random, \
                with 25.8% delegation overhead and roughly 17\u{00D7} slower end-to-end \
                execution.\n\n\
                The appendix also compares GPT-5 with GPT-4.1 under the same CoT policy. \
                GPT-5 gets better goal achievement, often around 0.6 to 0.7 on some \
                analytics and product-launch workflows, and uses a more proactive action \
                mix: 14.5\u{00D7} more decompositions, 7.8\u{00D7} more refinements, and \
                26\u{00D7} more dependency additions than GPT-4.1.",
        },
        TestCase {
            name: "bottom_line_bullets",
            markdown: "My read is that the best work is converging on a simple idea: multi-agent \
                systems help when they create real organizational structure, not when they just \
                create more chat.\n\nThe strongest recent papers do not support the naive story \
                that \"more agents = better reasoning.\" They support a narrower claim:\n\n\
                Teams help when agents have different roles, different information, different \
                tools, or different costs.\n\
                Teams help when coordination is architected explicitly: topology, handoff rules, \
                memory, verification, and stopping conditions.\n\
                Teams often hurt when they add conversational overhead, duplicate effort, or \
                weak verification.\n\
                The most promising human-in-the-loop designs are non-blocking control surfaces: \
                breakpoints, editable trajectories, approval gates, and post-hoc debugging, not \
                constant human babysitting.",
        },
        TestCase {
            name: "verification_centered",
            markdown: "The failure-analysis work makes verification look like the real bottleneck.\n\n\
                The MAST paper shows that many systems fail not because they cannot generate \
                candidate actions, but because they do not reliably check whether those actions \
                actually satisfy the task. This lines up with a broader pattern in agent systems: \
                generation is cheap, but trustworthy termination is hard.\n\n\
                A useful contrast is *Agentless: Demystifying LLM-based Software Engineering \
                Agents*. It is not a multi-agent paper, but it matters here because it shows \
                that a much simpler, interpretable three-stage pipeline can outperform more \
                elaborate autonomous agents on SWE-bench Lite. That is a critical baseline result.\n\n\
                So one of the most promising research directions is not \"add debate.\" It is:\n\n\
                - strong checkers,\n\
                - executable tests,\n\
                - formal or symbolic validators where possible,\n\
                - explicit completion criteria,\n\
                - confidence-aware escalation.\n\n\
                Your ToM seed fits here too. Beliefs and mental-state models are only valuable \
                if they feed better verification, handoff, or conflict resolution.",
        },
        TestCase {
            name: "practical_design_recipe",
            markdown: "If you wanted to build a genuinely strong human-in-the-loop multi-agent \
                system today, my recommended recipe would be:\n\n\
                1. Use a planner to create a task graph and artifact contract for each subtask.\n\
                2. Use a coordinator/router to assign subtasks to specialized workers.\n\
                3. Give workers narrow scopes and tool ownership.\n\
                4. Pass forward artifacts, not full chat logs, whenever possible.\n\
                5. Add a verifier that checks task completion against executable or symbolic criteria.\n\
                6. Insert human checkpoints only at plan approval, dangerous action approval, or \
                unresolved verification failure.\n\
                7. Log everything in a trajectory UI that supports breakpoints, replay, editing, \
                and branch comparison.\n\
                8. Evaluate against a single-agent baseline and a simple pipeline baseline before \
                claiming multi-agent value.\n\n\
                That recipe is basically where the strongest papers are pointing once you strip \
                away the branding differences.",
        },
        TestCase {
            name: "what_will_win",
            markdown: "If the goal is effective multi-agent systems in the next couple of years, \
                I would bet on this stack:\n\n\
                - Small hierarchical teams, not large peer-to-peer swarms.\n\
                - Planner-coordinator-worker-verifier patterns, with narrow worker scopes.\n\
                - Explicit shared state via artifacts, scratchpads, or blackboards, rather than \
                relying on agents to remember each other's context perfectly.\n\
                - Topology chosen for the task: relay chains for long context, DAGs for parallel \
                decomposition, approval loops for risky actions.\n\
                - Human control at breakpoints, not at every step.\n\
                - Strong evaluation against simple baselines, including single-agent and agentless \
                pipelines.\n\n\
                What I would currently avoid betting on:\n\n\
                - many-agent freeform debate as a default,\n\
                - persona-heavy role play without hard interfaces,\n\
                - systems that rely on implicit conversational coordination alone,\n\
                - papers that claim gains without reporting orchestration cost or strong baselines.",
        },
        TestCase {
            name: "attention_equation",
            markdown: "For each token, the model creates three vectors: query (Q), key (K), and value (V).\n\n\
                1. Compare a token's query with all keys to get relevance scores.\n\
                2. Normalize those scores with softmax to get weights.\n\
                3. Use the weights to blend the value vectors.\n\n\
                A compact form is:\n\
                <eq latex=\"\\\\mathrm{Attention}(Q,K,V)=\\\\mathrm{softmax}\\\\left(\\\\frac{QK^T}{\\\\sqrt{dk}}\\\\right)V\" display=\"block\">\
                attention equals softmax of Q K transpose over square root d k, then multiplied by V</eq>\n\n\
                Intuition: query asks \"what am I looking for?\", keys say \"what information do I contain?\", \
                and values are \"the information to pass forward.\"",
        },
        TestCase {
            name: "bullet_list_with_equals",
            markdown: "My read is that the best work is converging on a simple idea: multi-agent systems help when they create real organizational structure, not when they just create more chat.\n\nThe strongest recent papers do not support the naive story that \"more agents = better reasoning.\" They support a narrower claim:\n\nTeams help when agents have different roles, different information, different tools, or different costs.\nTeams help when coordination is architected explicitly: topology, handoff rules, memory, verification, and stopping conditions.\nTeams often hurt when they add conversational overhead, duplicate effort, or weak verification.",
        },
        TestCase {
            name: "sentence_boundary_sync",
            markdown: "This literature splits into four overlapping lines of work.\n\n\
                The first asks how choreography is actually made. Those papers treat \
                dance creation as an embodied design process rather than a purely \
                symbolic one. They study how choreographers improvise, constrain, \
                react to partners, externalize timing, and turn vague musical or \
                kinaesthetic ideas into repeatable phrases.",
        },
    ]
}

// ---------------------------------------------------------------------------
// TTS helper: exact production path (send_with_pauses + tts_worker_loop)
// ---------------------------------------------------------------------------

/// Mirrors the production `send_with_pauses` → `tts_worker_loop` flow exactly:
/// one WebSocket, sentences split on `[PAUSE:N]` markers, text chunks sent
/// via `send_text + flush`, pauses via `tokio::time::sleep`, EOS at the end.
async fn collect_all_chunks(config: &ElevenLabsConfig, sentences: &[String]) -> Vec<TtsChunk> {
    let mut stream = TtsStream::connect(config)
        .await
        .expect("failed to connect to ElevenLabs");

    for sentence in sentences {
        // Exact replica of production send_with_pauses + worker processing.
        let pause_marker = "[PAUSE:";
        let mut remaining = sentence.as_str();
        while let Some(start) = remaining.find(pause_marker) {
            let before = remaining[..start].trim();
            if !before.is_empty() {
                stream.send_text(before).await.expect("send_text");
                stream.flush().await.expect("flush");
            }
            let after_marker = &remaining[start + pause_marker.len()..];
            if let Some(end) = after_marker.find(']') {
                let ms: u64 = after_marker[..end].parse().unwrap_or(500).clamp(100, 3000);
                stream.flush().await.expect("flush before pause");
                tokio::time::sleep(tokio::time::Duration::from_millis(ms)).await;
                remaining = &after_marker[end + 1..];
            } else {
                break;
            }
        }
        let tail = remaining.trim();
        if !tail.is_empty() {
            stream.send_text(tail).await.expect("send_text");
            stream.flush().await.expect("flush");
        }
    }

    stream.send_eos().await;

    let mut chunks = Vec::new();
    while let Some(chunk) = stream.recv_audio().await {
        chunks.push(chunk);
    }
    chunks
}

// ---------------------------------------------------------------------------
// Run a single test case, append report section
// ---------------------------------------------------------------------------

async fn run_test_case(tc: &TestCase, config: &ElevenLabsConfig, report: &mut String) {
    let _ = writeln!(report, "# Test: {}\n", tc.name);
    let _ = writeln!(report, "**Input markdown:**\n```\n{}\n```\n", tc.markdown);

    // ── Phase 1: Text preparation ──────────────────────────────────────

    let _ = writeln!(report, "## Phase 1 -- Text preparation\n");

    let markup = browser_read_aloud_markup(tc.markdown);
    // Production calls clean_for_tts_preserving_equation_markers again in
    // on_voice_narrate_section (redundant but we match exactly).
    let cleaned = clean_for_tts_preserving_equation_markers(&markup);
    let _ = writeln!(
        report,
        "**Markup after preparation:**\n```\n{cleaned}\n```\n"
    );

    let (tts_text, eq_spans) = parse_equation_markers(&cleaned);
    let _ = writeln!(report, "**Clean TTS text:**\n```\n{tts_text}\n```\n");

    let _ = writeln!(report, "**Equation spans:** `{eq_spans:?}`\n");

    let mut sb = SentenceBuffer::new();
    let mut sentences: Vec<String> = sb.push(&tts_text);
    if let Some(last) = sb.flush() {
        sentences.push(last);
    }

    let _ = writeln!(report, "**Sentences ({}):**", sentences.len());
    for (i, s) in sentences.iter().enumerate() {
        let _ = writeln!(report, "  {i}. `{s}`");
    }
    let _ = writeln!(report);

    // ── Phase 2: TTS ──────────────────────────────────────────────────

    let _ = writeln!(report, "## Phase 2 -- TTS\n");

    let mut all_pcm: Vec<i16> = Vec::new();
    let mut timeline: Vec<AlignmentEntry> = Vec::new();
    let mut pending_word: Option<AlignmentEntry> = None;

    let chunks = collect_all_chunks(config, &sentences).await;
    for chunk in &chunks {
        all_pcm.extend_from_slice(&chunk.pcm);
        if let Some(ref align) = chunk.alignment {
            build_alignment_entries(align, 0, &mut timeline, &mut pending_word);
        }
    }

    // Flush any remaining pending word.
    if let Some(pw) = pending_word.take() {
        timeline.push(pw);
    }

    // Repair timestamp resets between text segments (matches production).
    repair_timeline_monotonicity(&mut timeline);

    let _ = writeln!(
        report,
        "**Timeline ({} words, {} PCM samples = {:.1}s at 24kHz):**\n",
        timeline.len(),
        all_pcm.len(),
        all_pcm.len() as f64 / 24000.0,
    );
    let _ = writeln!(report, "| # | Word | start_ms | duration_ms | eq |");
    let _ = writeln!(report, "|---|------|----------|-------------|-----|");

    for (i, entry) in timeline.iter().enumerate() {
        // Check if this word index falls inside an equation span.
        let eq_label = eq_spans
            .iter()
            .find(|(_, s, e)| i >= *s && i < *e)
            .map(|(idx, _, _)| format!("EQ{idx}"))
            .unwrap_or_default();

        let _ = writeln!(
            report,
            "| {i} | {} | {} | {} | {eq_label} |",
            entry.word, entry.start_ms, entry.duration_ms,
        );
    }
    let _ = writeln!(report);

    // ── Phase 3: STT ──────────────────────────────────────────────────

    let _ = writeln!(report, "## Phase 3 -- STT\n");

    if all_pcm.is_empty() {
        let _ = writeln!(report, "*No audio to transcribe.*\n");
    } else {
        let wav = pcm_to_wav(&all_pcm, 24000);
        // Save WAV for interactive karaoke demo
        let wav_path = format!("/tmp/tts-sync-{}.wav", tc.name);
        let _ = std::fs::write(&wav_path, &wav);
        let _ = writeln!(report, "**Audio saved:** `{wav_path}`\n");
        // Save timeline as JSON for interactive demo
        let demo_words: Vec<_> = timeline
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let eq_label = eq_spans
                    .iter()
                    .find(|(_, s, end)| i >= *s && i < *end)
                    .map(|(idx, _, _)| format!("EQ{idx}"))
                    .unwrap_or_default();
                serde_json::json!({
                    "idx": i,
                    "word": e.word,
                    "startMs": e.start_ms,
                    "durationMs": e.duration_ms,
                    "eq": eq_label,
                })
            })
            .collect();
        let demo_path = format!("/tmp/tts-sync-{}.json", tc.name);
        let _ = std::fs::write(
            &demo_path,
            serde_json::to_string_pretty(&demo_words).unwrap_or_default(),
        );
        let _ = writeln!(report, "**Demo data:** `{demo_path}`\n");
        match codex_elevenlabs::stt::transcribe(config, wav).await {
            Ok(transcript) => {
                let _ = writeln!(report, "**Transcript:**\n> {transcript}\n");
            }
            Err(e) => {
                let _ = writeln!(report, "**STT error:** `{e}`\n");
            }
        }
    }

    // ── Phase 4: Browser karaoke mapping ──────────────────────────────
    // Matches ensure_tts_playback_started: strip_pause_markers then count.

    let _ = writeln!(report, "## Phase 4 -- Browser karaoke mapping\n");

    let spoken_total_words = strip_pause_markers(&tts_text).split_whitespace().count();
    let hidden_equation_words: usize = eq_spans.iter().map(|(_, s, e)| e - s).sum();
    let visible_total_words = spoken_total_words.saturating_sub(hidden_equation_words);
    let _ = writeln!(
        report,
        "**Word counts:** spoken={spoken_total_words}, hidden_eq={hidden_equation_words}, visible={visible_total_words}\n"
    );

    let total_spoken_words = timeline.len();
    let _ = writeln!(report, "| spoken_idx | visible_idx | word | eq_highlight |");
    let _ = writeln!(report, "|------------|-------------|------|-------------|");

    for spoken_idx in 0..total_spoken_words {
        // Compute visible_word_idx by subtracting hidden equation words.
        // Uses (spoken_idx + 1) so the current word counts as hidden
        // when it's inside an equation (matches production fix).
        let mut hidden: usize = 0;
        for &(_eq_idx, start, end) in &eq_spans {
            if start > spoken_idx {
                break;
            }
            let overlap_end = (spoken_idx + 1).min(end);
            if overlap_end > start {
                hidden += overlap_end - start;
            }
        }
        let visible_idx = spoken_idx - hidden;

        // Active equation.
        let eq_highlight = eq_spans
            .iter()
            .find(|(_, s, e)| spoken_idx >= *s && spoken_idx < *e)
            .map(|(idx, _, _)| format!("EQ{idx}"))
            .unwrap_or_else(|| "-".to_string());

        let word = timeline
            .get(spoken_idx)
            .map(|e| e.word.as_str())
            .unwrap_or("?");

        let _ = writeln!(
            report,
            "| {spoken_idx} | {visible_idx} | {word} | {eq_highlight} |",
        );
    }
    let _ = writeln!(report);

    // ── Phase 5: TUI highlight walk ──────────────────────────────────

    let _ = writeln!(report, "## Phase 5 -- TUI highlight walk\n");

    let total_ms = timeline
        .last()
        .map(|e| e.start_ms + e.duration_ms + 100)
        .unwrap_or(0);

    let _ = writeln!(report, "| time_ms | word_idx | word |");
    let _ = writeln!(report, "|---------|----------|------|");

    let mut prev_idx: Option<usize> = None;
    let mut transitions = 0;
    let mut t: u64 = 0;
    while t <= total_ms && transitions < 20 {
        let active = find_active_word(&timeline, t);
        if active != prev_idx {
            let word = active
                .and_then(|i| timeline.get(i))
                .map(|e| e.word.as_str())
                .unwrap_or("(none)");
            let idx_str = active
                .map(|i| i.to_string())
                .unwrap_or_else(|| "-".to_string());
            let _ = writeln!(report, "| {t} | {idx_str} | {word} |");
            prev_idx = active;
            transitions += 1;
        }
        t += 10;
    }
    let _ = writeln!(report);

    // ── Phase 6: Alignment-driven matching proof ──────────────────────
    //
    // Simulates the proposed architecture: use alignment words as the
    // word list, then match each against the DOM text (approximated by
    // the clean display text with LaTeX→space). Shows whether every
    // spoken word has a corresponding DOM match.

    let _ = writeln!(
        report,
        "## Phase 6 -- Alignment-driven DOM matching (proof of concept)\n"
    );

    // Build "DOM text" — what the browser renders as visible text.
    // LaTeX $...$ → space (skipped by _walkAndWrap), markdown stripped.
    // This is an approximation; the real DOM comes from marked.js + KaTeX.
    let dom_text = {
        let mut s = markup.clone();
        // Strip display LaTeX $$...$$
        while let Some(start) = s.find("$$") {
            if let Some(end) = s[start + 2..].find("$$") {
                s = format!("{} {}", &s[..start], &s[start + 2 + end + 2..]);
            } else {
                break;
            }
        }
        // Strip inline LaTeX $...$
        while let Some(start) = s.find('$') {
            if let Some(end) = s[start + 1..].find('$') {
                s = format!("{} {}", &s[..start], &s[start + 1 + end + 1..]);
            } else {
                break;
            }
        }
        // Strip [[[EQ:N]]]...[[[/EQ]]] markers (equation paraphrases not in DOM)
        while let Some(start) = s.find("[[[EQ:") {
            if let Some(end) = s[start..].find("[[[/EQ]]]") {
                s = format!("{}{}", &s[..start], &s[start + end + 9..]);
            } else {
                break;
            }
        }
        // Strip [PAUSE:N] markers (TTS timing hints, not in rendered DOM)
        while let Some(start) = s.find("[PAUSE:") {
            if let Some(end) = s[start..].find(']') {
                s = format!("{}{}", &s[..start], &s[start + end + 1..]);
            } else {
                break;
            }
        }
        // Strip markdown formatting
        s = s.replace("**", "");
        s = s.replace('*', "");
        // Strip list markers (rendered as <li> in DOM, not text)
        let mut lines: Vec<&str> = s.lines().collect();
        lines = lines
            .iter()
            .map(|l| {
                let trimmed = l.trim_start();
                if trimmed.starts_with("- ")
                    || trimmed.starts_with("* ")
                    || trimmed.starts_with("+ ")
                {
                    &trimmed[2..]
                } else if let Some(rest) = trimmed
                    .strip_prefix(|c: char| c.is_ascii_digit())
                    .and_then(|r| r.strip_prefix(". "))
                {
                    rest
                } else {
                    l
                }
            })
            .collect();
        s = lines.join("\n");
        s
    };
    let dom_words: Vec<&str> = dom_text.split_whitespace().collect();
    let alignment_words: Vec<&str> = timeline.iter().map(|e| e.word.as_str()).collect();

    // Try to match each alignment word to the DOM text in order.
    // Non-equation words should appear in the DOM; equation words won't.
    let _ = writeln!(
        report,
        "**Alignment words:** {}\n**DOM words:** {}\n",
        alignment_words.len(),
        dom_words.len()
    );

    let _ = writeln!(
        report,
        "| align_idx | align_word | dom_match | dom_idx | status |"
    );
    let _ = writeln!(
        report,
        "|----------|-----------|----------|---------|--------|"
    );

    let mut dom_cursor = 0usize;
    let mut matched = 0usize;
    let mut eq_skipped = 0usize;
    for (ai, aword) in alignment_words.iter().enumerate() {
        let is_eq = eq_spans
            .iter()
            .any(|(_, start, end)| ai >= *start && ai < *end);
        if is_eq {
            let _ = writeln!(report, "| {ai} | {aword} | (eq paraphrase) | - | EQ_SKIP |");
            eq_skipped += 1;
            continue;
        }
        // Try to find this word in DOM text starting from cursor
        let found = dom_words[dom_cursor..]
            .iter()
            .position(|dw| {
                // Fuzzy match: strip trailing/leading punctuation for comparison
                let a = aword.trim_matches(|c: char| !c.is_alphanumeric());
                let d = dw.trim_matches(|c: char| !c.is_alphanumeric());
                a.eq_ignore_ascii_case(d) || *dw == *aword
            })
            .map(|pos| dom_cursor + pos);

        if let Some(di) = found {
            let _ = writeln!(
                report,
                "| {ai} | {aword} | {} | {di} | MATCH |",
                dom_words[di]
            );
            dom_cursor = di + 1;
            matched += 1;
        } else {
            let _ = writeln!(report, "| {ai} | {aword} | ??? | - | **MISS** |");
        }
    }

    let _ = writeln!(report);
    let _ = writeln!(
        report,
        "**Summary:** {matched} matched, {eq_skipped} eq_skipped, {} missed out of {} alignment words\n",
        alignment_words.len() - matched - eq_skipped,
        alignment_words.len()
    );

    // ── Phase 7: Interrupt test ──────────────────────────────────────

    let _ = writeln!(report, "## Phase 7 -- Interrupt test\n");

    // Reconnect and send all sentences (same as production restart).
    let chunks2 = collect_all_chunks(config, &sentences).await;

    let mut timeline2: Vec<AlignmentEntry> = Vec::new();
    let mut pending2: Option<AlignmentEntry> = None;
    for chunk in &chunks2 {
        if let Some(ref align) = chunk.alignment {
            build_alignment_entries(align, 0, &mut timeline2, &mut pending2);
        }
    }
    if let Some(pw) = pending2.take() {
        timeline2.push(pw);
    }

    let first_start = timeline2.first().map(|e| e.start_ms).unwrap_or(u64::MAX);
    let word_count_match = timeline2.len() == timeline.len();

    let _ = writeln!(
        report,
        "- First word starts at: **{first_start}ms** (expected near 0)",
    );
    let _ = writeln!(
        report,
        "- Word count: **{}** vs original **{}** -- {}",
        timeline2.len(),
        timeline.len(),
        if word_count_match {
            "MATCH"
        } else {
            "MISMATCH"
        },
    );
    let _ = writeln!(report);
    let _ = writeln!(report, "---\n");
}

// ---------------------------------------------------------------------------
// Main test
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn generate_tts_sync_report() {
    let api_key = std::env::var("ELEVENLABS_API_KEY")
        .expect("ELEVENLABS_API_KEY must be set to run this test");
    assert!(!api_key.is_empty(), "ELEVENLABS_API_KEY is empty");

    let config = ElevenLabsConfig::new(api_key);

    let mut report = String::new();
    let _ = writeln!(report, "# TTS / Karaoke Sync Report\n");
    let _ = writeln!(
        report,
        "Generated: {}\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    );
    let _ = writeln!(report, "---\n");

    let cases = test_cases();
    for tc in &cases {
        eprintln!("=== Running test case: {} ===", tc.name);
        run_test_case(tc, &config, &mut report).await;
    }

    let path = "/tmp/tts-sync-report.md";
    std::fs::write(path, &report).expect("failed to write report");
    eprintln!("\nReport written to: {path}");
}

/// Run a single test case by name. Usage:
/// ```sh
/// cargo test -p codex-tui --features voice-input --test tts_sync_report -- --ignored single -- real_section_state_equation
/// ```
#[tokio::test]
#[ignore]
async fn single() {
    let api_key = std::env::var("ELEVENLABS_API_KEY")
        .expect("ELEVENLABS_API_KEY must be set to run this test");
    assert!(!api_key.is_empty(), "ELEVENLABS_API_KEY is empty");

    let config = ElevenLabsConfig::new(api_key);

    // Pick test case from env or default to the last one.
    let target = std::env::var("TTS_TEST_CASE").unwrap_or_default();
    let cases = test_cases();
    let tc = if target.is_empty() {
        cases.last().expect("no test cases")
    } else {
        cases
            .iter()
            .find(|c| c.name == target)
            .unwrap_or_else(|| panic!("test case '{target}' not found"))
    };

    let mut report = String::new();
    let _ = writeln!(report, "# TTS Single Test: {}\n", tc.name);
    eprintln!("=== Running: {} ===", tc.name);
    run_test_case(tc, &config, &mut report).await;

    let path = "/tmp/tts-sync-single.md";
    std::fs::write(path, &report).expect("failed to write report");
    eprintln!("\nReport written to: {path}");
}
