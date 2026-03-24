#!/usr/bin/env python3
"""Analyze ATA session rollout files for context pollution and token usage.

Usage:
  analyze_rollout.py <rollout.jsonl>           # Analyze a specific rollout file
  analyze_rollout.py latest                    # Analyze the most recent rollout
  analyze_rollout.py latest --parent           # Analyze the most recent parent (non-subagent) rollout
  analyze_rollout.py --session-dir <dir>       # Analyze all rollouts in a session dir
  analyze_rollout.py --list                    # List recent sessions

Reports:
  - Token usage breakdown by category (system, skills, tools, user, assistant, tool outputs)
  - Context accumulation over turns
  - Skill injection sizes
  - Tool call patterns
  - Subagent spawn/wait patterns
  - Staging document production tracking
"""

import argparse
import json
import os
import re
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from glob import glob
from pathlib import Path
from typing import Optional


# Rough token estimation: ~4 chars per token for English text
CHARS_PER_TOKEN = 4

ATA_SESSIONS_DIR = os.path.expanduser("~/.ata/sessions")


@dataclass
class ContentBlock:
    """A categorized piece of content in the conversation."""
    category: str  # system, skill, tool_desc, user, assistant, tool_call, tool_output, reasoning
    subcategory: str  # e.g., skill name, tool name
    char_count: int
    estimated_tokens: int
    turn: int
    line_index: int


@dataclass
class TurnInfo:
    """Aggregated info for one turn."""
    turn_id: str = ""
    model: str = ""
    effort: str = ""
    input_tokens: int = 0
    cached_tokens: int = 0
    output_tokens: int = 0
    reasoning_tokens: int = 0
    context_window: int = 0


@dataclass
class ToolCallInfo:
    """Info about a tool call."""
    name: str = ""
    call_id: str = ""
    args_len: int = 0
    args_preview: str = ""
    output_len: int = 0
    turn: int = 0


@dataclass
class SubagentInfo:
    """Info about a subagent spawn."""
    agent_type: str = ""
    prompt_preview: str = ""
    turn: int = 0
    staging_file_produced: bool = False


@dataclass
class RolloutAnalysis:
    """Complete analysis of a rollout file."""
    file_path: str = ""
    session_id: str = ""
    model_provider: str = ""
    source: str = ""

    blocks: list = field(default_factory=list)
    turns: list = field(default_factory=list)
    tool_calls: list = field(default_factory=list)
    subagents: list = field(default_factory=list)

    total_lines: int = 0
    is_subagent: bool = False
    subagent_role: str = ""
    subagent_nickname: str = ""
    skill_injections: dict = field(default_factory=dict)  # skill_name -> char count


def estimate_tokens(text: str) -> int:
    """Rough token estimate from character count."""
    return max(1, len(text) // CHARS_PER_TOKEN)


def categorize_developer_content(text: str) -> tuple[str, str]:
    """Categorize a developer message part into (category, subcategory)."""
    text_lower = text[:500].lower()
    first_line = text.split("\n")[0].strip()

    # Skill injection via <skill> XML tag (structured injection from ATA)
    if "<skill>" in text[:100] or "<name>" in text[:100]:
        name_match = re.search(r"<name>([^<]+)</name>", text[:300])
        if name_match:
            return "skill", name_match.group(1)

    # Skill injection detection — skills start with "# Title" and contain
    # instruction-like content
    if first_line.startswith("# "):
        heading = first_line.lstrip("# ").strip()
        has_instruction_markers = any(marker in text_lower for marker in [
            "## instructions", "## rules", "## the flow", "## core rules",
            "## execution", "## when to use", "## common workflows",
            "your tools:", "subagent", "spawn", "staging file",
            "**your tools:**", "do not call", "## kb path",
        ])
        known_skills = [
            "paper synth", "hn synth", "hn discover", "zotero", "knowledge base",
            "cross-paper", "conversation report", "paper discovery",
            "hacker news", "synthesizer", "workspace",
        ]
        is_known = any(kw in heading.lower() for kw in known_skills)
        if has_instruction_markers or is_known:
            return "skill", heading

    # AGENTS.md / user instructions
    if "# agents.md" in text_lower or "agents.md instructions" in text_lower:
        return "system", "agents_md"

    # Permissions instructions
    if "<permissions instructions>" in text_lower or "sandbox" in text_lower[:200]:
        return "system", "permissions"

    # Base instructions / personality
    if "you are codex" in text_lower or "personality" in text_lower[:300]:
        return "system", "base_instructions"

    # User instructions (CLAUDE.md etc)
    if "user instructions" in text_lower or "claude.md" in text_lower:
        return "system", "user_instructions"

    # Tool descriptions
    if "tool:" in text_lower[:50] or "function" in text_lower[:50]:
        return "tool_desc", "tool_registration"

    # Environment context
    if "<environment_context>" in text or "current date" in text_lower[:200] or "timezone" in text_lower[:100]:
        return "system", "environment"

    # Skill invocation marker (e.g., "$paper-synthesizer\n\nPaper: ...")
    if text.startswith("$") or ("$" in text[:30] and len(text) < 500):
        return "skill", "invocation"

    # Default system
    return "system", "other"


def _extract_args_preview(tool_name: str, args: str) -> str:
    """Extract a short human-readable preview of tool call arguments."""
    if not args:
        return ""
    try:
        obj = json.loads(args) if isinstance(args, str) else args
    except (json.JSONDecodeError, TypeError):
        return args[:70]

    if not isinstance(obj, dict):
        return str(obj)[:70]

    # Show first few key=value pairs, truncating long values
    parts = []
    for k, v in list(obj.items())[:3]:
        sv = str(v)
        if len(sv) > 50:
            sv = sv[:47] + "..."
        parts.append(f"{k}={sv}")
    return ", ".join(parts)[:80]


def analyze_rollout(filepath: str) -> RolloutAnalysis:
    """Analyze a single rollout JSONL file."""
    analysis = RolloutAnalysis(file_path=filepath)

    current_turn = 0
    current_turn_info = TurnInfo()

    with open(filepath) as f:
        lines = f.readlines()

    analysis.total_lines = len(lines)

    for line_idx, line in enumerate(lines):
        try:
            obj = json.loads(line.strip())
        except json.JSONDecodeError:
            continue

        evt_type = obj.get("type", "")
        payload = obj.get("payload", {})

        if evt_type == "session_meta":
            analysis.session_id = payload.get("id", "")
            analysis.model_provider = payload.get("model_provider", "")
            analysis.source = payload.get("source", "")

            # Check if this is a subagent session
            source = payload.get("source", "")
            if isinstance(source, dict) and "subagent" in source:
                analysis.is_subagent = True
                sub_info = source.get("subagent", {}).get("thread_spawn", {})
                analysis.subagent_role = sub_info.get("agent_role", "")
                analysis.subagent_nickname = sub_info.get("agent_nickname", "")
            elif isinstance(source, str) and source == "subagent":
                analysis.is_subagent = True

        elif evt_type == "turn_context":
            current_turn += 1
            current_turn_info = TurnInfo(
                turn_id=payload.get("turn_id", ""),
                model=payload.get("model", ""),
                effort=payload.get("effort", ""),
            )
            analysis.turns.append(current_turn_info)

        elif evt_type == "response_item":
            item_type = payload.get("type", "")
            role = payload.get("role", "")

            if item_type == "message":
                content = payload.get("content", [])
                if isinstance(content, list):
                    for part in content:
                        if not isinstance(part, dict):
                            continue
                        text = part.get("text", "")
                        if not text:
                            continue

                        char_count = len(text)
                        est_tokens = estimate_tokens(text)

                        if role == "developer":
                            cat, subcat = categorize_developer_content(text)
                            if cat == "skill":
                                analysis.skill_injections[subcat] = (
                                    analysis.skill_injections.get(subcat, 0) + char_count
                                )
                        elif role == "user":
                            cat, subcat = "user", "message"
                        elif role == "assistant":
                            cat, subcat = "assistant", "message"
                        else:
                            cat, subcat = "other", role or "unknown"

                        analysis.blocks.append(ContentBlock(
                            category=cat,
                            subcategory=subcat,
                            char_count=char_count,
                            estimated_tokens=est_tokens,
                            turn=current_turn,
                            line_index=line_idx,
                        ))

                elif isinstance(content, str):
                    char_count = len(content)
                    if role == "user":
                        cat, subcat = "user", "message"
                    elif role == "assistant":
                        cat, subcat = "assistant", "message"
                    else:
                        cat, subcat = "other", role or "unknown"

                    analysis.blocks.append(ContentBlock(
                        category=cat,
                        subcategory=subcat,
                        char_count=char_count,
                        estimated_tokens=estimate_tokens(content),
                        turn=current_turn,
                        line_index=line_idx,
                    ))

            elif item_type == "function_call":
                name = payload.get("name", "")
                call_id = payload.get("call_id", "")
                args = payload.get("arguments", "")
                if isinstance(args, dict):
                    args = json.dumps(args)

                args_preview = _extract_args_preview(name, args)
                tc = ToolCallInfo(
                    name=name,
                    call_id=call_id,
                    args_len=len(args) if args else 0,
                    args_preview=args_preview,
                    output_len=0,
                    turn=current_turn,
                )
                analysis.tool_calls.append(tc)

                analysis.blocks.append(ContentBlock(
                    category="tool_call",
                    subcategory=name,
                    char_count=len(args) if args else 0,
                    estimated_tokens=estimate_tokens(args) if args else 0,
                    turn=current_turn,
                    line_index=line_idx,
                ))

                # Detect subagent spawns
                if name in ("spawn_agent", "agent"):
                    try:
                        args_obj = json.loads(args) if isinstance(args, str) else args
                        agent_type = args_obj.get("agent_type", args_obj.get("type", ""))
                        prompt = args_obj.get("prompt", "")[:200]
                        analysis.subagents.append(SubagentInfo(
                            agent_type=agent_type,
                            prompt_preview=prompt,
                            turn=current_turn,
                        ))
                    except (json.JSONDecodeError, TypeError):
                        pass

            elif item_type == "function_call_output":
                call_id = payload.get("call_id", "")
                output = payload.get("output", "")
                output_len = len(output) if isinstance(output, str) else 0

                # Match to tool call to get the tool name
                matched_tool_name = "unknown"
                for tc in analysis.tool_calls:
                    if tc.call_id == call_id:
                        tc.output_len = output_len
                        matched_tool_name = tc.name
                        break

                analysis.blocks.append(ContentBlock(
                    category="tool_output",
                    subcategory=matched_tool_name,
                    char_count=output_len,
                    estimated_tokens=estimate_tokens(output) if isinstance(output, str) else 0,
                    turn=current_turn,
                    line_index=line_idx,
                ))

                # Check for staging file mentions in output
                if isinstance(output, str) and "staging/" in output:
                    for sa in analysis.subagents:
                        if not sa.staging_file_produced:
                            sa.staging_file_produced = True
                            break

            elif item_type == "reasoning":
                # Reasoning tokens (not in context but still billed)
                summary = payload.get("summary", [])
                text = ""
                if isinstance(summary, list):
                    text = " ".join(s.get("text", "") for s in summary if isinstance(s, dict))
                analysis.blocks.append(ContentBlock(
                    category="reasoning",
                    subcategory="model_reasoning",
                    char_count=len(text),
                    estimated_tokens=estimate_tokens(text),
                    turn=current_turn,
                    line_index=line_idx,
                ))

        elif evt_type == "event_msg":
            msg_type = payload.get("type", "")
            if msg_type == "token_count" and payload.get("info"):
                info = payload["info"]
                total = info.get("total_token_usage", {})
                last = info.get("last_token_usage", {})

                if current_turn_info:
                    current_turn_info.input_tokens = total.get("input_tokens", 0)
                    current_turn_info.cached_tokens = total.get("cached_input_tokens", 0)
                    current_turn_info.output_tokens = total.get("output_tokens", 0)
                    current_turn_info.reasoning_tokens = total.get("reasoning_output_tokens", 0)
                    current_turn_info.context_window = info.get("model_context_window", 0)

    return analysis


def format_tokens(n: int) -> str:
    """Format token count with K suffix."""
    if n >= 1000:
        return f"{n/1000:.1f}K"
    return str(n)


def format_chars(n: int) -> str:
    """Format character count."""
    if n >= 1000000:
        return f"{n/1000000:.1f}M"
    if n >= 1000:
        return f"{n/1000:.1f}K"
    return str(n)


def print_report(analysis: RolloutAnalysis, verbose: bool = False):
    """Print a formatted analysis report."""
    print("=" * 70)
    print(f"  ATA Session Rollout Analysis")
    print(f"  File: {os.path.basename(analysis.file_path)}")
    print(f"  Session: {analysis.session_id[:20]}...")
    print(f"  Provider: {analysis.model_provider} | Source: {analysis.source}")
    if analysis.is_subagent:
        role_info = f" (role={analysis.subagent_role}, nick={analysis.subagent_nickname})" if analysis.subagent_role else ""
        print(f"  Subagent: Yes{role_info}")
    else:
        print(f"  Subagent: No")
    print(f"  Total events: {analysis.total_lines} | Turns: {len(analysis.turns)}")
    print("=" * 70)

    # Category breakdown
    category_chars = defaultdict(int)
    category_tokens = defaultdict(int)
    subcategory_chars = defaultdict(lambda: defaultdict(int))

    for block in analysis.blocks:
        category_chars[block.category] += block.char_count
        category_tokens[block.category] += block.estimated_tokens
        subcategory_chars[block.category][block.subcategory] += block.char_count

    total_chars = sum(category_chars.values())
    total_tokens = sum(category_tokens.values())

    print(f"\n{'CATEGORY BREAKDOWN':^70}")
    print("-" * 70)
    print(f"  {'Category':<20} {'Chars':>10} {'~Tokens':>10} {'% of Total':>12}")
    print("-" * 70)

    for cat in sorted(category_chars.keys(), key=lambda c: -category_chars[c]):
        chars = category_chars[cat]
        tokens = category_tokens[cat]
        pct = (chars / total_chars * 100) if total_chars > 0 else 0
        bar = "#" * int(pct / 2)
        print(f"  {cat:<20} {format_chars(chars):>10} {format_tokens(tokens):>10} {pct:>6.1f}% {bar}")

        if verbose:
            for subcat in sorted(subcategory_chars[cat].keys(),
                                  key=lambda s: -subcategory_chars[cat][s]):
                sc = subcategory_chars[cat][subcat]
                sc_tokens = estimate_tokens("x" * sc)
                sc_pct = (sc / total_chars * 100) if total_chars > 0 else 0
                print(f"    {subcat:<18} {format_chars(sc):>10} {format_tokens(sc_tokens):>10} {sc_pct:>6.1f}%")

    print("-" * 70)
    print(f"  {'TOTAL':<20} {format_chars(total_chars):>10} {format_tokens(total_tokens):>10} {'100.0%':>7}")

    # Skill injections
    if analysis.skill_injections:
        print(f"\n{'SKILL INJECTIONS':^70}")
        print("-" * 70)
        print(f"  {'Skill Name':<35} {'Chars':>10} {'~Tokens':>10}")
        print("-" * 70)
        total_skill_chars = 0
        for name in sorted(analysis.skill_injections.keys(),
                           key=lambda n: -analysis.skill_injections[n]):
            chars = analysis.skill_injections[name]
            total_skill_chars += chars
            tokens = estimate_tokens("x" * chars)
            print(f"  {name:<35} {format_chars(chars):>10} {format_tokens(tokens):>10}")
        print("-" * 70)
        print(f"  {'TOTAL SKILL CONTENT':<35} {format_chars(total_skill_chars):>10} {format_tokens(estimate_tokens('x'*total_skill_chars)):>10}")
        if total_chars > 0:
            print(f"  Skills as % of total context: {total_skill_chars/total_chars*100:.1f}%")

    # Tool call summary
    if analysis.tool_calls:
        print(f"\n{'TOOL CALLS':^70}")
        print("-" * 70)
        tool_summary = defaultdict(lambda: {"count": 0, "args_chars": 0, "output_chars": 0})
        for tc in analysis.tool_calls:
            tool_summary[tc.name]["count"] += 1
            tool_summary[tc.name]["args_chars"] += tc.args_len
            tool_summary[tc.name]["output_chars"] += tc.output_len

        print(f"  {'Tool':<25} {'Calls':>6} {'Args':>10} {'Output':>10} {'~Out Tok':>10}")
        print("-" * 70)
        for name in sorted(tool_summary.keys(), key=lambda n: -tool_summary[n]["output_chars"]):
            s = tool_summary[name]
            print(f"  {name:<25} {s['count']:>6} {format_chars(s['args_chars']):>10} {format_chars(s['output_chars']):>10} {format_tokens(estimate_tokens('x'*s['output_chars'])):>10}")

        total_output = sum(s["output_chars"] for s in tool_summary.values())
        print("-" * 70)
        print(f"  Total tool output: {format_chars(total_output)} (~{format_tokens(estimate_tokens('x'*total_output))} tokens)")
        if total_chars > 0:
            print(f"  Tool outputs as % of total context: {total_output/total_chars*100:.1f}%")

    # Tool output breakdown — per-tool aggregation + individual large outputs
    if analysis.tool_calls:
        # Aggregate by tool name
        tool_out = defaultdict(lambda: {"count": 0, "total": 0, "outputs": []})
        for tc in analysis.tool_calls:
            if tc.output_len > 0:
                tool_out[tc.name]["count"] += 1
                tool_out[tc.name]["total"] += tc.output_len
                tool_out[tc.name]["outputs"].append((tc.output_len, tc.args_preview))

        total_output = sum(v["total"] for v in tool_out.values())

        if total_output > 0:
            print(f"\n{'TOOL OUTPUT BREAKDOWN':^70}")
            print("-" * 70)
            print(f"  {'Tool':<22} {'Calls':>5} {'Total':>9} {'~Tokens':>8} {'Avg':>8} {'Max':>8} {'%':>6}")
            print("-" * 70)

            for name in sorted(tool_out.keys(), key=lambda n: -tool_out[n]["total"]):
                v = tool_out[name]
                avg = v["total"] // v["count"]
                mx = max(o[0] for o in v["outputs"])
                pct = v["total"] / total_output * 100
                print(f"  {name:<22} {v['count']:>5} {format_chars(v['total']):>9} {format_tokens(estimate_tokens('x'*v['total'])):>8} {format_chars(avg):>8} {format_chars(mx):>8} {pct:>5.1f}%")

            print("-" * 70)
            print(f"  Total: {format_chars(total_output)} (~{format_tokens(estimate_tokens('x'*total_output))} tokens)")

            # Show individual large outputs (top N by size)
            all_outputs = []
            for tc in analysis.tool_calls:
                if tc.output_len > 0:
                    all_outputs.append(tc)
            all_outputs.sort(key=lambda x: -x.output_len)

            top_n = min(15, len(all_outputs))
            if top_n > 0 and all_outputs[0].output_len > 2000:
                print(f"\n  Top {top_n} largest individual outputs:")
                print(f"  {'#':>3} {'Tool':<22} {'Chars':>9} {'~Tokens':>8}  Context")
                print(f"  {'-'*67}")
                for i, tc in enumerate(all_outputs[:top_n]):
                    ctx = tc.args_preview[:50] if tc.args_preview else ""
                    print(f"  {i+1:>3} {tc.name:<22} {format_chars(tc.output_len):>9} {format_tokens(estimate_tokens('x'*tc.output_len)):>8}  {ctx}")

    # Subagent tracking
    if analysis.subagents:
        print(f"\n{'SUBAGENT SPAWNS':^70}")
        print("-" * 70)
        for i, sa in enumerate(analysis.subagents):
            staging = "YES" if sa.staging_file_produced else "NO"
            print(f"  [{i+1}] type={sa.agent_type} staging={staging}")
            print(f"      prompt: {sa.prompt_preview[:60]}...")

    # Turn-by-turn token usage (from API)
    api_turns = [t for t in analysis.turns if t.input_tokens > 0]
    if api_turns:
        print(f"\n{'TURN-BY-TURN TOKEN USAGE (API)':^70}")
        print("-" * 70)
        print(f"  {'Turn':>5} {'Model':<25} {'Input':>8} {'Cached':>8} {'Output':>8} {'Reason':>8}")
        print("-" * 70)
        for i, t in enumerate(api_turns):
            model_short = t.model[:24] if t.model else "?"
            print(f"  {i+1:>5} {model_short:<25} {format_tokens(t.input_tokens):>8} {format_tokens(t.cached_tokens):>8} {format_tokens(t.output_tokens):>8} {format_tokens(t.reasoning_tokens):>8}")

        last = api_turns[-1]
        print("-" * 70)
        print(f"  Final totals: input={format_tokens(last.input_tokens)} cached={format_tokens(last.cached_tokens)} output={format_tokens(last.output_tokens)} reasoning={format_tokens(last.reasoning_tokens)}")
        if last.context_window > 0:
            utilization = last.input_tokens / last.context_window * 100
            print(f"  Context window: {format_tokens(last.context_window)} | Utilization: {utilization:.1f}%")

    # Context accumulation chart (text-based)
    if len(analysis.turns) > 1:
        print(f"\n{'CONTEXT ACCUMULATION':^70}")
        print("-" * 70)
        turn_cumulative = defaultdict(lambda: defaultdict(int))
        for block in analysis.blocks:
            if block.turn > 0:
                turn_cumulative[block.turn][block.category] += block.estimated_tokens

        # Accumulate
        cats = sorted(set(b.category for b in analysis.blocks))
        running = {c: 0 for c in cats}
        max_tokens = 0
        turn_data = []

        for turn in sorted(turn_cumulative.keys()):
            for cat in cats:
                running[cat] += turn_cumulative[turn].get(cat, 0)
            total = sum(running.values())
            max_tokens = max(max_tokens, total)
            turn_data.append((turn, dict(running), total))

        if max_tokens > 0:
            bar_width = 40
            for turn, cats_tokens, total in turn_data:
                bar = ""
                for cat in ["system", "skill", "tool_desc", "user", "assistant", "tool_call", "tool_output", "reasoning"]:
                    if cat in cats_tokens and cats_tokens[cat] > 0:
                        seg_len = max(1, int(cats_tokens[cat] / max_tokens * bar_width))
                        char = {"system": "S", "skill": "K", "tool_desc": "D", "user": "U",
                                "assistant": "A", "tool_call": "C", "tool_output": "O", "reasoning": "R",
                                "other": "?"}.get(cat, "?")
                        bar += char * seg_len
                print(f"  T{turn:>2} [{bar:<{bar_width}}] {format_tokens(total)}")
            print()
            print("  Legend: S=system K=skill D=tool_desc U=user A=assistant C=tool_call O=tool_output R=reasoning")

    # Warnings
    print(f"\n{'WARNINGS':^70}")
    print("-" * 70)
    warnings = []

    # Check for large skill injections
    for name, chars in analysis.skill_injections.items():
        if chars > 5000:
            warnings.append(f"Large skill injection: {name} ({format_chars(chars)} chars, ~{format_tokens(estimate_tokens('x'*chars))} tokens)")

    # Check tool output dominance
    tool_output_chars = category_chars.get("tool_output", 0)
    if total_chars > 0 and tool_output_chars / total_chars > 0.5:
        warnings.append(f"Tool outputs dominate context ({tool_output_chars/total_chars*100:.0f}% of total)")

    # Check for subagents without staging files
    for i, sa in enumerate(analysis.subagents):
        if not sa.staging_file_produced:
            warnings.append(f"Subagent [{i+1}] ({sa.agent_type}) did NOT produce a staging file")

    # Check for duplicate skill injections
    seen_skills = defaultdict(int)
    for block in analysis.blocks:
        if block.category == "skill":
            seen_skills[block.subcategory] += 1
    for name, count in seen_skills.items():
        if count > 1:
            warnings.append(f"Skill '{name}' injected {count} times (possible duplication)")

    if warnings:
        for w in warnings:
            print(f"  ! {w}")
    else:
        print("  No warnings.")

    print("\n" + "=" * 70)


def find_latest_rollout(parent_only: bool = False) -> Optional[str]:
    """Find the most recent rollout file."""
    pattern = os.path.join(ATA_SESSIONS_DIR, "**", "rollout-*.jsonl")
    files = glob(pattern, recursive=True)
    if not files:
        return None

    files.sort(key=os.path.getmtime, reverse=True)

    if not parent_only:
        return files[0]

    # Find the latest non-subagent rollout
    for f in files:
        with open(f) as fh:
            first_line = fh.readline()
            try:
                obj = json.loads(first_line)
                payload = obj.get("payload", {})
                source = payload.get("source", "")
                if source in ("cli", "tui"):
                    return f
            except json.JSONDecodeError:
                continue

    return files[0] if files else None


def list_sessions(limit: int = 20):
    """List recent sessions."""
    pattern = os.path.join(ATA_SESSIONS_DIR, "**", "rollout-*.jsonl")
    files = glob(pattern, recursive=True)
    files.sort(key=os.path.getmtime, reverse=True)

    print(f"{'Recent ATA Sessions':^70}")
    print("-" * 70)
    print(f"  {'#':>3} {'Date':>12} {'Size':>8} {'Events':>7} {'File'}")
    print("-" * 70)

    for i, f in enumerate(files[:limit]):
        size = os.path.getsize(f)
        with open(f) as fh:
            event_count = sum(1 for _ in fh)
        basename = os.path.basename(f)
        # Extract date from filename
        date_match = re.search(r'(\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2})', basename)
        date_str = date_match.group(1).replace("T", " ").replace("-", ":", 2) if date_match else "?"

        print(f"  {i+1:>3} {date_str:>12} {format_chars(size):>8} {event_count:>7} {basename}")


def main():
    parser = argparse.ArgumentParser(description="Analyze ATA session rollout files")
    parser.add_argument("rollout", nargs="?", help="Path to rollout JSONL file, or 'latest'")
    parser.add_argument("--parent", action="store_true", help="When using 'latest', find the latest parent (non-subagent) rollout")
    parser.add_argument("--session-dir", help="Analyze all rollouts in a session directory")
    parser.add_argument("--list", action="store_true", help="List recent sessions")
    parser.add_argument("-v", "--verbose", action="store_true", help="Show subcategory breakdown")
    parser.add_argument("--json", action="store_true", help="Output as JSON instead of text")

    args = parser.parse_args()

    if args.list:
        list_sessions()
        return

    if args.session_dir:
        pattern = os.path.join(args.session_dir, "rollout-*.jsonl")
        files = sorted(glob(pattern))
        if not files:
            print(f"No rollout files found in {args.session_dir}")
            sys.exit(1)
        for f in files:
            analysis = analyze_rollout(f)
            print_report(analysis, verbose=args.verbose)
        return

    if not args.rollout:
        parser.print_help()
        sys.exit(1)

    if args.rollout == "latest":
        filepath = find_latest_rollout(parent_only=args.parent)
        if not filepath:
            print("No rollout files found")
            sys.exit(1)
    else:
        filepath = args.rollout

    if not os.path.exists(filepath):
        print(f"File not found: {filepath}")
        sys.exit(1)

    analysis = analyze_rollout(filepath)

    if args.json:
        # JSON output for programmatic use
        output = {
            "file": analysis.file_path,
            "session_id": analysis.session_id,
            "model_provider": analysis.model_provider,
            "is_subagent": analysis.is_subagent,
            "total_events": analysis.total_lines,
            "turns": len(analysis.turns),
            "categories": {},
            "skill_injections": analysis.skill_injections,
            "tool_calls": [
                {"name": tc.name, "args_chars": tc.args_len, "output_chars": tc.output_len, "args_preview": tc.args_preview}
                for tc in analysis.tool_calls
            ],
            "subagents": [
                {"type": sa.agent_type, "staging_produced": sa.staging_file_produced}
                for sa in analysis.subagents
            ],
        }

        for block in analysis.blocks:
            cat = block.category
            if cat not in output["categories"]:
                output["categories"][cat] = {"chars": 0, "estimated_tokens": 0}
            output["categories"][cat]["chars"] += block.char_count
            output["categories"][cat]["estimated_tokens"] += block.estimated_tokens

        print(json.dumps(output, indent=2))
    else:
        print_report(analysis, verbose=args.verbose)


if __name__ == "__main__":
    main()
