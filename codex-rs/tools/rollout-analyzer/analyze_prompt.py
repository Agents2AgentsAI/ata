#!/usr/bin/env python3
"""Analyze ATA prompt snapshot files for ground-truth token usage.

Usage:
  analyze_prompt.py latest              # Most recent session
  analyze_prompt.py <snapshot.jsonl>    # Specific file
  analyze_prompt.py --list              # List recent snapshots
  analyze_prompt.py --compare A B       # Compare two snapshot files
  analyze_prompt.py latest --detail 3   # Show full content of API call #3
  analyze_prompt.py latest --detail all # Show all API calls with full content
"""

import argparse
import json
import os
import sys
from glob import glob

ATA_SESSIONS_DIR = os.path.expanduser("~/.ata/sessions")


def find_latest():
    pattern = os.path.join(ATA_SESSIONS_DIR, "**", "prompt-snapshot-*.jsonl")
    files = glob(pattern, recursive=True)
    if not files:
        return None
    return max(files, key=os.path.getmtime)


def list_snapshots(limit=20):
    pattern = os.path.join(ATA_SESSIONS_DIR, "**", "prompt-snapshot-*.jsonl")
    files = sorted(glob(pattern, recursive=True), key=os.path.getmtime, reverse=True)

    print(f"{'#':>3} {'Time':>8} {'Calls':>5} {'Total':>7} {'ToolOut':>7} {'Dev':>5} {'User':>6}  File")
    print("-" * 80)

    for i, path in enumerate(files[:limit]):
        with open(path) as f:
            lines = f.readlines()
        if not lines:
            continue
        last = json.loads(lines[-1])
        from datetime import datetime
        dt = datetime.fromtimestamp(os.path.getmtime(path)).strftime("%H:%M")
        total = _total(last)
        print(f"{i+1:>3} {dt:>8} {len(lines):>5} {total:>7} {_cat(last,'tool_output'):>7} {_cat(last,'developer'):>5} {_cat(last,'user'):>6}  {os.path.basename(path)}")


def _cat(d, name):
    for c in d.get("breakdown", []):
        if c["category"] == name:
            return c.get("tokens", c.get("chars", 0) // 4)
    return 0


def _total(d):
    if "total_prompt_tokens" in d:
        return d["total_prompt_tokens"]
    base = d.get("base_instructions_tokens", d.get("base_instructions_chars", 0) // 4)
    tools = d.get("tools_tokens", d.get("tools_chars", 0) // 4)
    inp = d.get("total_input_tokens", d.get("total_input_chars", 0) // 4)
    return base + tools + inp


def analyze(path):
    with open(path) as f:
        lines = f.readlines()

    if not lines:
        print("Empty snapshot file.")
        return

    entries = [json.loads(line) for line in lines]
    last = entries[-1]
    first = entries[0]

    print("=" * 65)
    print(f"  Prompt Snapshot: {os.path.basename(path)}")
    print(f"  Model: {first.get('model', '?')}")
    print(f"  API calls: {len(entries)}")
    print("=" * 65)

    # Per-call summary
    print(f"\n{'CALL-BY-CALL':^65}")
    print("-" * 65)
    print(f"  {'#':>3} {'Base':>6} {'Tools':>6} {'Dev':>6} {'User':>6} {'Asst':>5} {'TCall':>5} {'TOut':>6} {'TOTAL':>7}")
    print("-" * 65)

    for i, e in enumerate(entries):
        base = e.get("base_instructions_tokens", e.get("base_instructions_chars", 0) // 4)
        tools = e.get("tools_tokens", e.get("tools_chars", 0) // 4)
        total = _total(e)
        print(f"  {i+1:>3} {base:>6} {tools:>6} {_cat(e,'developer'):>6} {_cat(e,'user'):>6} {_cat(e,'assistant'):>5} {_cat(e,'tool_call'):>5} {_cat(e,'tool_output'):>6} {total:>7}")

    # Final call breakdown
    print(f"\n{'FINAL CALL BREAKDOWN':^65}")
    print("-" * 65)

    base = last.get("base_instructions_tokens", last.get("base_instructions_chars", 0) // 4)
    tools_tok = last.get("tools_tokens", last.get("tools_chars", 0) // 4)
    tools_n = last.get("tools_count", 0)
    total = _total(last)

    rows = [
        ("Base instructions", base),
        (f"Tool definitions ({tools_n} tools)", tools_tok),
    ]
    for c in last.get("breakdown", []):
        tok = c.get("tokens", c.get("chars", 0) // 4)
        rows.append((f"{c['category']} ({c['count']} items)", tok))

    print(f"  {'Component':<35} {'Tokens':>8} {'%':>6}")
    print(f"  {'-'*50}")
    for label, tok in rows:
        pct = tok * 100 // total if total > 0 else 0
        bar = "#" * (pct // 2)
        print(f"  {label:<35} {tok:>8} {pct:>5}% {bar}")
    print(f"  {'-'*50}")
    print(f"  {'TOTAL':<35} {total:>8}")

    # Growth chart
    if len(entries) > 1:
        print(f"\n{'CONTEXT GROWTH':^65}")
        print("-" * 65)
        max_total = max(_total(e) for e in entries)
        bar_width = 40
        for i, e in enumerate(entries):
            t = _total(e)
            bar_len = t * bar_width // max_total if max_total > 0 else 0
            tool_out = _cat(e, "tool_output")
            tool_pct = tool_out * bar_width // max_total if max_total > 0 else 0
            other = bar_len - tool_pct
            bar = "=" * other + "O" * tool_pct
            print(f"  {i+1:>2} [{bar:<{bar_width}}] {t:>6}")
        print(f"\n  = fixed context  O = tool outputs")


def _find_rollout_for_snapshot(snapshot_path):
    """Find the matching rollout file in the same directory."""
    directory = os.path.dirname(snapshot_path)
    # snapshot: prompt-snapshot-<session-id>.jsonl
    # rollout:  rollout-<timestamp>-<session-id>.jsonl
    basename = os.path.basename(snapshot_path)
    session_id = basename.replace("prompt-snapshot-", "").replace(".jsonl", "")
    for f in os.listdir(directory):
        if f.startswith("rollout-") and session_id in f:
            return os.path.join(directory, f)
    return None


def _parse_rollout_items(rollout_path):
    """Parse rollout into a list of conversation items in order."""
    items = []
    with open(rollout_path) as f:
        for line in f:
            try:
                obj = json.loads(line.strip())
            except json.JSONDecodeError:
                continue
            if obj.get("type") != "response_item":
                continue
            p = obj.get("payload", {})
            item_type = p.get("type", "")
            role = p.get("role", "")

            if item_type == "message":
                content = p.get("content", [])
                texts = []
                if isinstance(content, list):
                    for part in content:
                        if isinstance(part, dict) and "text" in part:
                            texts.append(part["text"])
                elif isinstance(content, str):
                    texts.append(content)
                total_chars = sum(len(t) for t in texts)
                items.append({
                    "kind": "message",
                    "role": role,
                    "chars": total_chars,
                    "parts": len(texts),
                    "texts": texts,
                })
            elif item_type == "function_call":
                name = p.get("name", "")
                args = p.get("arguments", "")
                if isinstance(args, dict):
                    args = json.dumps(args)
                items.append({
                    "kind": "tool_call",
                    "name": name,
                    "chars": len(args),
                    "args": args,
                })
            elif item_type == "function_call_output":
                output = p.get("output", "")
                output_text = output if isinstance(output, str) else json.dumps(output)
                items.append({
                    "kind": "tool_output",
                    "chars": len(output_text),
                    "text": output_text,
                })
            elif item_type == "reasoning":
                items.append({"kind": "reasoning", "chars": 0})
    return items


def _truncate(text, max_len=500):
    if len(text) <= max_len:
        return text
    return text[:max_len] + f"\n... ({len(text)} chars total, showing first {max_len})"


def detail(snapshot_path, call_number):
    """Show full content for a specific API call or all calls."""
    rollout_path = _find_rollout_for_snapshot(snapshot_path)
    if not rollout_path:
        print(f"No matching rollout file found for {os.path.basename(snapshot_path)}")
        print("Looking in:", os.path.dirname(snapshot_path))
        return

    with open(snapshot_path) as f:
        snapshots = [json.loads(line) for line in f]

    items = _parse_rollout_items(rollout_path)

    if not snapshots:
        print("Empty snapshot file.")
        return

    # Each snapshot corresponds to an API call. The items up to snapshot[i].input_items_count
    # are what was sent in call i. Since items accumulate, call i includes all items from
    # previous calls plus new ones.

    show_all = call_number == "all"

    for call_idx, snap in enumerate(snapshots):
        call_num = call_idx + 1
        if not show_all and str(call_num) != str(call_number):
            continue

        n_items = snap.get("input_items_count", 0)
        total = _total(snap)

        print("=" * 70)
        print(f"  API CALL #{call_num}  |  {n_items} items  |  ~{total} tokens")
        print(f"  Model: {snap.get('model', '?')}")
        print("=" * 70)

        # Show base instructions size
        base_tok = snap.get("base_instructions_tokens", snap.get("base_instructions_chars", 0) // 4)
        print(f"\n  [BASE INSTRUCTIONS] ~{base_tok} tokens")

        # Show tools
        tools_tok = snap.get("tools_tokens", snap.get("tools_chars", 0) // 4)
        tools_n = snap.get("tools_count", 0)
        print(f"  [TOOL DEFINITIONS] {tools_n} tools, ~{tools_tok} tokens")

        # Show the conversation items that make up this call
        # Items are cumulative — call N has items 0..n_items
        call_items = items[:n_items] if n_items <= len(items) else items

        print(f"\n  --- Conversation ({len(call_items)} items) ---\n")

        for j, item in enumerate(call_items):
            kind = item["kind"]
            chars = item["chars"]
            tokens_est = chars // 4

            if kind == "message":
                role = item["role"]
                parts = item["parts"]
                header = f"  [{j+1}] {role} message ({parts} parts, ~{tokens_est} tokens)"
                print(header)
                print(f"  {'~' * (len(header) - 2)}")
                for k, text in enumerate(item["texts"]):
                    if parts > 1:
                        print(f"  [part {k+1}/{parts}, {len(text)} chars]")
                    print(f"  {_truncate(text, 400)}")
                    print()

            elif kind == "tool_call":
                name = item["name"]
                print(f"  [{j+1}] tool_call: {name} (~{tokens_est} tokens)")
                print(f"  {'~' * 40}")
                print(f"  {_truncate(item['args'], 300)}")
                print()

            elif kind == "tool_output":
                print(f"  [{j+1}] tool_output (~{tokens_est} tokens)")
                print(f"  {'~' * 40}")
                print(f"  {_truncate(item['text'], 400)}")
                print()

            elif kind == "reasoning":
                print(f"  [{j+1}] reasoning")
                print()

        if not show_all:
            # Show nav hint
            print(f"  Showing call {call_num}/{len(snapshots)}.", end="")
            if call_num > 1:
                print(f" Previous: --detail {call_num - 1}", end="")
            if call_num < len(snapshots):
                print(f" Next: --detail {call_num + 1}", end="")
            print(f" All: --detail all")

    if not show_all and call_number not in [str(i + 1) for i in range(len(snapshots))]:
        print(f"Call #{call_number} not found. This session has {len(snapshots)} API calls (1-{len(snapshots)}).")


def compare(path_a, path_b):
    with open(path_a) as f:
        a_entries = [json.loads(line) for line in f]
    with open(path_b) as f:
        b_entries = [json.loads(line) for line in f]

    if not a_entries or not b_entries:
        print("One or both files are empty.")
        return

    a_last = a_entries[-1]
    b_last = b_entries[-1]

    print("=" * 70)
    print(f"  COMPARISON")
    print(f"  A: {os.path.basename(path_a)}")
    print(f"  B: {os.path.basename(path_b)}")
    print("=" * 70)

    def pct(old, new):
        if old == 0:
            return "N/A"
        return f"{(new - old) * 100 // old:+d}%"

    a_base = a_last.get("base_instructions_tokens", a_last.get("base_instructions_chars", 0) // 4)
    b_base = b_last.get("base_instructions_tokens", b_last.get("base_instructions_chars", 0) // 4)
    a_tools = a_last.get("tools_tokens", a_last.get("tools_chars", 0) // 4)
    b_tools = b_last.get("tools_tokens", b_last.get("tools_chars", 0) // 4)
    a_total = _total(a_last)
    b_total = _total(b_last)

    rows = [
        ("API calls", len(a_entries), len(b_entries)),
        ("Total prompt tokens", a_total, b_total),
        ("Base instructions", a_base, b_base),
        ("Tool definitions", a_tools, b_tools),
    ]

    cats = set()
    for c in a_last.get("breakdown", []) + b_last.get("breakdown", []):
        cats.add(c["category"])
    for cat in sorted(cats):
        rows.append((cat, _cat(a_last, cat), _cat(b_last, cat)))

    print(f"\n  {'Metric':<30} {'A':>8} {'B':>8} {'Delta':>8}")
    print(f"  {'-'*55}")
    for label, a_val, b_val in rows:
        print(f"  {label:<30} {a_val:>8} {b_val:>8} {pct(a_val, b_val):>8}")


def main():
    parser = argparse.ArgumentParser(description="Analyze ATA prompt snapshots")
    parser.add_argument("snapshot", nargs="?", help="Path or 'latest'")
    parser.add_argument("--list", action="store_true", help="List recent snapshots")
    parser.add_argument("--compare", nargs=2, metavar=("A", "B"), help="Compare two snapshots")
    parser.add_argument("--detail", metavar="N", help="Show full content of API call N (or 'all')")

    args = parser.parse_args()

    if args.list:
        list_snapshots()
        return

    if args.compare:
        a, b = args.compare
        if a == "latest":
            a = find_latest()
        if b == "latest":
            b = find_latest()
        if not a or not b:
            print("Snapshot file(s) not found")
            sys.exit(1)
        compare(a, b)
        return

    if not args.snapshot:
        parser.print_help()
        sys.exit(1)

    path = args.snapshot
    if path == "latest":
        path = find_latest()
        if not path:
            print("No snapshot files found")
            sys.exit(1)

    if not os.path.exists(path):
        print(f"File not found: {path}")
        sys.exit(1)

    if args.detail:
        detail(path, args.detail)
    else:
        analyze(path)


if __name__ == "__main__":
    main()
