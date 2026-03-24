---
name: hn-discoverer
description: "INTERNAL SUBAGENT SKILL — never invoke directly. This is called automatically by $hn-synthesis to discover relevant HN threads. If a user asks about HN discussions, use $hn-synthesis instead."
metadata:
  short-description: "[Internal] Subagent skill for hn-synthesis discovery"
policy:
  allow_implicit_invocation: false
---

# HN Discoverer

Search Hacker News for threads relevant to the given topic, deduplicate, rank, and return top thread IDs.

## Instructions

1. Run multiple `hn_search` calls in parallel to maximize coverage — vary `sort_by` (relevance, date), `min_points` (5, 50), and `content_type` (story, comment).
2. Use synonyms and related terms for broad topics (e.g., "AI agent" / "LLM agent" / "agentic").
3. Use `date_from`/`date_to` (YYYY-MM-DD) when user specifies a timeframe.
4. Deduplicate by `object_id`. Appearing in multiple searches = positive signal, weight higher.
5. Rank by: relevance, points, comment count (prefer 10+ comments over high-point low-comment), recency.
6. Return **top 3-7 threads**.

**Do NOT call** `spawn_agent`, `present_reading_view`, `hn_get_thread`, or any file tools. Your only tool is `hn_search`.

## Return Format

```
THREAD <story_id> | <points> pts | <num_comments> comments | <title> | <hn_url> | <linked_url>
```

`<linked_url>`: the article/blog/paper URL, or `(self)` for Ask HN / text posts. Include 1-sentence topic context per thread.
