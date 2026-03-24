---
name: hn-synthesizer
description: "INTERNAL SUBAGENT SKILL — never invoke directly. This is called automatically by $hn-synthesis when it spawns a thread-analysis subagent. If a user asks about HN discussions, use $hn-synthesis instead."
metadata:
  short-description: "[Internal] Subagent skill for hn-synthesis"
policy:
  allow_implicit_invocation: false
---

# HN Synthesizer

Retrieve ONE Hacker News thread, extract discussion insights, write a staging file. **You MUST write the staging file — this is your only deliverable. Without it, the main agent cannot proceed.**

## Instructions

1. **`hn_get_thread`** with the given thread ID: `hn_get_thread(item_id: "<story_id>", max_depth: 8, max_comments: 200)`.
2. Analyze the discussion (see below).
3. **Write staging file:**
   ```
   mkdir -p ${CODEX_KB_PATH}/staging && cat <<'CARD_EOF' > ${CODEX_KB_PATH}/staging/hn-<thread_id>.md
   ---
   thread_id: "<story_id>"
   title: "<thread title>"
   hn_url: "<HN URL>"
   article_url: "<linked URL or (self)>"
   points: <points>
   comments: <comment_count>
   date: "<YYYY-MM-DD>"
   author: "<author>"
   ---
   <your full extracted analysis>
   CARD_EOF
   ```
4. Return **only the staging file path**. Do NOT return full text.

**Do NOT call** `spawn_agent`, `present_reading_view`, `hn_search`, or file tools. Your tools are `hn_get_thread` and `exec_command` only.

## What to Extract

Capture specific details, quotes, and numbers — not vague summaries. The main agent needs rich source material.

- **Community sentiment** — characterize as overwhelmingly positive / mostly positive / mixed / mostly negative / controversial. What do supporters emphasize vs critics?
- **Key arguments for and against** — 2-3 sentences per argument. What practitioners like, what problems it solves, what criticisms and concerns arise.
- **Nuanced takes** — conditional opinions ("great for X, terrible for Y"), trade-off analyses, caveats.
- **Practitioner experience** — real-world deployment reports: who (role/context), what they tried, scale, outcome, specific numbers.
- **Resources surfaced** — links, repos, papers, alternative tools mentioned, with context for why.
- **Unresolved questions** — what the community raised but didn't answer.
