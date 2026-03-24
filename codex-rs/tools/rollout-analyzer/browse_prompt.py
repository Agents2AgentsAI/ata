#!/usr/bin/env python3
"""Interactive TUI browser for ATA prompt snapshots.

Matches the Prompt Inspector vim plugin style: collapsible tree sidebar
on the left, content preview on the right, token counts right-aligned.

Usage:
  browse_prompt.py latest
  browse_prompt.py <snapshot.jsonl>

Keys:
  j/k or ↑↓       Navigate
  Enter            Toggle category / scroll to item
  Backspace        Collapse category
  PgUp/PgDn        Scroll content
  s                Cycle sort (position → tokens)
  T                Toggle token counts
  q                Quit
  ?                Help
"""

import argparse
import curses
import json
import os
import sys
from glob import glob

ATA_SESSIONS_DIR = os.path.expanduser("~/.ata/sessions")


def find_latest():
    pattern = os.path.join(ATA_SESSIONS_DIR, "**", "prompt-snapshot-*.jsonl")
    files = glob(pattern, recursive=True)
    return max(files, key=os.path.getmtime) if files else None


def find_rollout(snapshot_path):
    directory = os.path.dirname(snapshot_path)
    session_id = os.path.basename(snapshot_path).replace("prompt-snapshot-", "").replace(".jsonl", "")
    for f in os.listdir(directory):
        if f.startswith("rollout-") and session_id in f:
            return os.path.join(directory, f)
    return None


def parse_rollout(rollout_path):
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
            it = p.get("type", "")
            role = p.get("role", "")
            if it == "message":
                content = p.get("content", [])
                texts = []
                if isinstance(content, list):
                    for part in content:
                        if isinstance(part, dict) and "text" in part:
                            texts.append(part["text"])
                elif isinstance(content, str):
                    texts.append(content)
                chars = sum(len(t) for t in texts)
                items.append({"kind": "message", "role": role, "chars": chars,
                              "tokens": chars // 4, "texts": texts,
                              "name": texts[0][:45].replace("\n", " ").strip() if texts else "(empty)"})
            elif it == "function_call":
                name = p.get("name", "")
                args = p.get("arguments", "")
                if isinstance(args, dict):
                    args = json.dumps(args, indent=2)
                items.append({"kind": "tool_call", "name": name, "chars": len(args),
                              "tokens": len(args) // 4, "text": args})
            elif it == "function_call_output":
                out = p.get("output", "")
                if not isinstance(out, str):
                    out = json.dumps(out, indent=2)
                items.append({"kind": "tool_output", "chars": len(out),
                              "tokens": len(out) // 4, "text": out, "name": f"output ({len(out)} chars)"})
            elif it == "reasoning":
                items.append({"kind": "reasoning", "chars": 0, "tokens": 0, "name": "reasoning"})
    return items


def _total(d):
    if "total_prompt_tokens" in d:
        return d["total_prompt_tokens"]
    return (d.get("base_instructions_tokens", d.get("base_instructions_chars", 0) // 4)
            + d.get("tools_tokens", d.get("tools_chars", 0) // 4)
            + d.get("total_input_tokens", d.get("total_input_chars", 0) // 4))


def _cat(d, name):
    for c in d.get("breakdown", []):
        if c["category"] == name:
            return c.get("tokens", c.get("chars", 0) // 4)
    return 0


def _fmt(n):
    return f"{n / 1000:.1f}k" if n >= 1000 else str(n)


def _item_detail(item):
    k = item["kind"]
    lines = []
    if k == "message":
        lines.append(f"{item['role'].upper()}  {item['chars']} chars  ~{_fmt(item['tokens'])} tok  {len(item['texts'])} parts")
        lines.append("")
        for i, t in enumerate(item["texts"]):
            if len(item["texts"]) > 1:
                lines.append(f"── part {i + 1}/{len(item['texts'])} ({len(t)} chars) ──")
                lines.append("")
            lines.extend(t.split("\n"))
            lines.append("")
    elif k == "tool_call":
        lines.append(f"CALL  {item['name']}  ~{_fmt(item['tokens'])} tok")
        lines.append("")
        try:
            lines.extend(json.dumps(json.loads(item["text"]), indent=2).split("\n"))
        except Exception:
            lines.extend(item["text"].split("\n"))
    elif k == "tool_output":
        lines.append(f"OUTPUT  {item['chars']} chars  ~{_fmt(item['tokens'])} tok")
        lines.append("")
        text = item["text"]
        # Try to pretty-print JSON outputs
        stripped = text.strip()
        if stripped.startswith("{") or stripped.startswith("["):
            try:
                lines.extend(json.dumps(json.loads(stripped), indent=2).split("\n"))
            except (json.JSONDecodeError, TypeError):
                lines.extend(text.split("\n"))
        else:
            lines.extend(text.split("\n"))
    elif k == "reasoning":
        lines.append("REASONING (content not captured)")
    return lines


class Browser:
    def __init__(self, snapshots, items, path):
        self.snapshots = snapshots
        self.path = path
        last = snapshots[-1] if snapshots else {}
        n = last.get("input_items_count", len(items))
        self.items = items[:n] if n <= len(items) else items
        self.sw = 38  # sidebar width — matches prompt inspector
        self.show_tokens = True
        self.sort_mode = "position"  # "position" | "tokens"

        # Categories: group items by role/kind
        self._build_categories()

        # Tree state
        self.expanded = set()  # expanded category names
        self.lines = []  # rendered sidebar lines
        self.line_map = []  # line -> {"type": "cat"/"entry", ...}
        self.cursor = 0
        self.scroll = 0

    def _build_categories(self):
        cats = {}  # name -> [items]
        order = []
        for item in self.items:
            k = item["kind"]
            if k == "message":
                cat = item["role"]
            else:
                cat = k
            if cat not in cats:
                cats[cat] = []
                order.append(cat)
            cats[cat].append(item)

        self.categories = []
        for name in order:
            entries = cats[name]
            total_tok = sum(e.get("tokens", 0) for e in entries)
            self.categories.append({"name": name, "entries": entries, "total_tokens": total_tok})

    def _sorted_categories(self):
        if self.sort_mode == "tokens":
            return sorted(self.categories, key=lambda c: -c["total_tokens"])
        return self.categories  # position order

    def _render_tree(self):
        self.lines = []
        self.line_map = []

        for cat in self._sorted_categories():
            name = cat["name"]
            count = len(cat["entries"])
            expanded = name in self.expanded
            arrow = "▼" if expanded else "▶"

            tok_str = f"{_fmt(cat['total_tokens'])}t" if self.show_tokens else ""
            label = f"{arrow} {name}  ({count})"
            self.lines.append((label, tok_str, "cat"))
            self.line_map.append({"type": "cat", "name": name})

            if expanded:
                entries = cat["entries"]
                if self.sort_mode == "tokens":
                    entries = sorted(entries, key=lambda e: -e.get("tokens", 0))
                for entry in entries:
                    ename = entry.get("name", "?")[:self.sw - 12]
                    etok = f"{_fmt(entry.get('tokens', 0))}t" if self.show_tokens else ""
                    self.lines.append((f"  ● {ename}", etok, "entry"))
                    self.line_map.append({"type": "entry", "item": entry, "cat": name})

        # Footer
        total = _total(self.snapshots[-1]) if self.snapshots else 0
        self.lines.append(("", "", "blank"))
        self.line_map.append({"type": "blank"})
        foot = f"~{_fmt(total)} tok  {len(self.items)} items"
        self.lines.append((foot, "", "footer"))
        self.line_map.append({"type": "footer"})
        sort_line = f"Sort: {self.sort_mode} | s=cycle T=tok ?=help"
        self.lines.append((sort_line, "", "footer"))
        self.line_map.append({"type": "footer"})

    def _get_detail(self):
        if self.cursor >= len(self.line_map):
            return [""]
        info = self.line_map[self.cursor]
        if info["type"] == "entry":
            return _item_detail(info["item"])
        if info["type"] == "cat":
            name = info["name"]
            cat = next((c for c in self.categories if c["name"] == name), None)
            if cat:
                lines = [f"{name.upper()}  {len(cat['entries'])} items  ~{_fmt(cat['total_tokens'])} tok", ""]
                for e in cat["entries"]:
                    lines.append(f"  ● {e.get('name', '?')[:50]}  ~{_fmt(e.get('tokens', 0))}t")
                return lines
        return ["Navigate to an entry to see its content."]

    def run(self, stdscr):
        curses.curs_set(0)
        curses.use_default_colors()
        if curses.has_colors():
            curses.init_pair(1, curses.COLOR_CYAN, -1)
            curses.init_pair(2, curses.COLOR_GREEN, -1)
            curses.init_pair(3, curses.COLOR_YELLOW, -1)
            curses.init_pair(4, curses.COLOR_RED, -1)
            curses.init_pair(5, curses.COLOR_BLACK, curses.COLOR_CYAN)
            curses.init_pair(6, curses.COLOR_MAGENTA, -1)

        self._render_tree()
        # Expand first category by default
        if self.categories:
            self.expanded.add(self.categories[0]["name"])
            self._render_tree()

        while True:
            stdscr.erase()
            h, w = stdscr.getmaxyx()
            dw = w - self.sw - 1

            # Sidebar
            vis_start = max(0, self.cursor - h // 2)
            for y in range(h - 1):
                idx = vis_start + y
                if idx >= len(self.lines):
                    break
                label, tok, kind = self.lines[idx]
                sel = idx == self.cursor

                # Color
                if sel:
                    attr = curses.color_pair(5) | curses.A_BOLD
                elif kind == "cat":
                    attr = curses.A_BOLD
                elif kind == "footer":
                    attr = curses.A_DIM
                elif kind == "blank":
                    attr = curses.A_NORMAL
                else:
                    info = self.line_map[idx]
                    item = info.get("item", {})
                    ik = item.get("kind", "")
                    role = item.get("role", "")
                    if role == "developer" or ik == "tool_call":
                        attr = curses.color_pair(3)
                    elif role == "user":
                        attr = curses.color_pair(2)
                    elif role == "assistant":
                        attr = curses.color_pair(1)
                    elif ik == "tool_output":
                        attr = curses.color_pair(4)
                    elif ik == "reasoning":
                        attr = curses.A_DIM
                    else:
                        attr = curses.A_NORMAL

                # Layout: label left, tokens right
                max_label = self.sw - len(tok) - 2
                display_label = label[:max_label]
                pad = self.sw - 1 - len(display_label) - len(tok)
                if pad < 1:
                    pad = 1
                line_str = display_label + " " * pad + tok

                try:
                    stdscr.addnstr(y, 0, line_str, self.sw - 1, attr)
                    # Dim the token count part
                    if tok and not sel:
                        tok_start = len(line_str) - len(tok)
                        stdscr.addnstr(y, tok_start, tok, len(tok), curses.A_DIM)
                except curses.error:
                    pass

            # Separator
            for y in range(h - 1):
                try:
                    stdscr.addch(y, self.sw, "│", curses.A_DIM)
                except curses.error:
                    pass

            # Detail pane
            detail = self._get_detail()
            max_sc = max(0, len(detail) - h + 2)
            self.scroll = min(self.scroll, max_sc)
            x = self.sw + 1
            for y in range(h - 1):
                li = self.scroll + y
                if li >= len(detail):
                    break
                try:
                    stdscr.addnstr(y, x, detail[li][:dw], dw)
                except curses.error:
                    pass

            # Status
            ve = min(self.scroll + h - 1, len(detail))
            status = f" ↑↓ navigate  Enter toggle  PgUp/Dn scroll  s sort  T tokens  ? help  q quit │ {self.cursor + 1}/{len(self.lines)} │ {self.scroll + 1}–{ve}/{len(detail)}"
            try:
                stdscr.addnstr(h - 1, 0, status.ljust(w), w, curses.A_REVERSE)
            except curses.error:
                pass

            stdscr.refresh()
            key = stdscr.getch()

            if key == ord("q"):
                break
            elif key in (curses.KEY_UP, ord("k")):
                self.cursor = max(0, self.cursor - 1)
                while self.cursor > 0 and self.line_map[self.cursor]["type"] == "blank":
                    self.cursor -= 1
                self.scroll = 0
            elif key in (curses.KEY_DOWN, ord("j")):
                self.cursor = min(len(self.lines) - 1, self.cursor + 1)
                while self.cursor < len(self.lines) - 1 and self.line_map[self.cursor]["type"] == "blank":
                    self.cursor += 1
                self.scroll = 0
            elif key in (curses.KEY_ENTER, ord("\n"), 10, 13):
                info = self.line_map[self.cursor]
                if info["type"] == "cat":
                    name = info["name"]
                    if name in self.expanded:
                        self.expanded.discard(name)
                    else:
                        self.expanded.add(name)
                    self._render_tree()
                self.scroll = 0
            elif key in (curses.KEY_BACKSPACE, 127, 8):
                info = self.line_map[self.cursor]
                cat_name = info.get("cat") or info.get("name")
                if cat_name and cat_name in self.expanded:
                    self.expanded.discard(cat_name)
                    self._render_tree()
                    # Move cursor to category header
                    for i, lm in enumerate(self.line_map):
                        if lm["type"] == "cat" and lm["name"] == cat_name:
                            self.cursor = i
                            break
                self.scroll = 0
            elif key == curses.KEY_PPAGE:
                self.scroll = max(0, self.scroll - (h - 2))
            elif key == curses.KEY_NPAGE:
                self.scroll += h - 2
            elif key == ord("g"):
                self.cursor = 0
                self.scroll = 0
            elif key == ord("G"):
                self.cursor = len(self.lines) - 1
                self.scroll = 0
            elif key == ord("s"):
                self.sort_mode = "tokens" if self.sort_mode == "position" else "position"
                self._render_tree()
            elif key == ord("T"):
                self.show_tokens = not self.show_tokens
                self._render_tree()
            elif key == ord("?"):
                self._show_help(stdscr)

    def _show_help(self, stdscr):
        h, w = stdscr.getmaxyx()
        help_lines = [
            " Prompt Snapshot Browser",
            " ───────────────────────",
            "  ↑↓ j/k   Navigate sidebar",
            "  Enter     Toggle category expand/collapse",
            "  Backspace Collapse current category",
            "  PgUp/PgDn Scroll content pane",
            "  s         Cycle sort (position ↔ tokens)",
            "  T         Toggle token count display",
            "  g/G       Jump to first/last",
            "  q         Quit",
            "",
            "  Press any key to close.",
        ]
        bh = len(help_lines) + 2
        bw = max(len(l) for l in help_lines) + 4
        sy = (h - bh) // 2
        sx = (w - bw) // 2
        win = curses.newwin(bh, bw, sy, sx)
        win.box()
        for i, line in enumerate(help_lines):
            try:
                win.addnstr(i + 1, 2, line, bw - 4)
            except curses.error:
                pass
        win.refresh()
        win.getch()


def main():
    parser = argparse.ArgumentParser(description="Browse prompt snapshots")
    parser.add_argument("snapshot", nargs="?", help="Path or 'latest'")
    args = parser.parse_args()
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
        print(f"Not found: {path}")
        sys.exit(1)
    rollout = find_rollout(path)
    if not rollout:
        print(f"No rollout for {os.path.basename(path)}")
        sys.exit(1)
    with open(path) as f:
        snaps = [json.loads(l) for l in f]
    items = parse_rollout(rollout)
    if not snaps:
        print("Empty snapshot.")
        sys.exit(1)
    Browser(snaps, items, path).run(curses.wrapper(lambda s: s) or None)


# Can't use curses.wrapper directly with a method, so:
def _main():
    parser = argparse.ArgumentParser(description="Browse prompt snapshots")
    parser.add_argument("snapshot", nargs="?", help="Path or 'latest'")
    args = parser.parse_args()
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
        print(f"Not found: {path}")
        sys.exit(1)
    rollout = find_rollout(path)
    if not rollout:
        print(f"No rollout for {os.path.basename(path)}")
        sys.exit(1)
    with open(path) as f:
        snaps = [json.loads(l) for l in f]
    items = parse_rollout(rollout)
    if not snaps:
        print("Empty snapshot.")
        sys.exit(1)
    b = Browser(snaps, items, path)
    curses.wrapper(b.run)


if __name__ == "__main__":
    _main()
