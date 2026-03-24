#!/usr/bin/env python3
"""Prompt Inspector — discovers, extracts, and enriches all agent-facing content.

Outputs structured JSON to stdout.

Usage:
    python3 generate.py --codex-root /path/to/codex-rs
    python3 generate.py --codex-root /path/to/codex-rs --metadata-only
    python3 generate.py --codex-root /path/to/codex-rs --entry "base instructions"
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import re
import subprocess
import sys
from datetime import datetime, timezone

from extractor import extract_file_content, extract_rust_const, extract_rust_function
from metadata import git_metadata_batch, token_count

# Try tomllib (3.11+) then tomli
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore[no-redef]


# ---------------------------------------------------------------------------
# Category mapping
# ---------------------------------------------------------------------------

# Maps relative directory prefixes to category names.
# Order matters: first match wins.
DIRECTORY_CATEGORY_MAP: list[tuple[str, str]] = [
    ("core/templates/collaboration_mode/", "Collaboration"),
    ("core/templates/memories/", "Memory System"),
    ("core/templates/research/", "Research"),
    ("core/templates/review/", "Review"),
    ("core/templates/compact/", "Compact"),
    ("core/templates/coordination/", "Coordination"),
    ("core/templates/agents/", "Agent Messages"),
    ("core/templates/collab/", "Collaboration"),
    ("core/templates/tools/", "Tool Descriptions"),
    ("core/templates/search_tool/", "Tool Descriptions"),
    ("core/templates/model_instructions/", "System Prompts"),
    ("core/templates/personalities/", "Personalities"),
    ("protocol/src/prompts/permissions/", "Permissions"),
    ("protocol/src/prompts/realtime/", "Realtime"),
    ("protocol/src/prompts/base_instructions/", "System Prompts"),
    ("core/src/tools/handlers/tool_", "Tool Descriptions"),
    ("core/src/agent/builtins/", "Agent Messages"),
    ("apply-patch/", "System Prompts"),
]

# Category slug to display name mapping
CATEGORY_SLUG_MAP = {
    "system-prompts": "System Prompts",
    "tool-descriptions": "Tool Descriptions",
    "agent-messages": "Agent Messages",
    "context-injection": "Context Injection",
}


def _categorize_path(relpath: str) -> str:
    """Determine category for a file based on its relative path."""
    for prefix, category in DIRECTORY_CATEGORY_MAP:
        if relpath.startswith(prefix):
            return category

    # Skills
    if "/assets/" in relpath and relpath.endswith("/SKILL.md"):
        return "Skills"
    if "/assets/" in relpath and "/references/" in relpath:
        return "Skill References"

    # Root-level gpt prompts
    if re.match(r"core/gpt.*\.md$", relpath):
        return "System Prompts"

    # Root-level .md files in core/ or tui/
    if re.match(r"(?:core|tui)/[^/]+\.md$", relpath):
        return "System Prompts"

    return "Uncategorized"


def _name_from_path(relpath: str) -> str:
    """Derive a human-readable name from a file path."""
    basename = os.path.basename(relpath)
    name, _ = os.path.splitext(basename)

    # For SKILL.md, use the parent directory name
    if basename == "SKILL.md":
        parts = relpath.split("/")
        # e.g. skills/src/assets/research/paper-discovery/SKILL.md -> paper-discovery
        if len(parts) >= 2:
            return parts[-2]
        return name

    return name.replace("_", " ").replace("-", " ")


# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------

GLOB_PATTERNS = [
    "core/templates/**/*.md",
    "protocol/src/prompts/**/*.md",
    "skills/src/assets/**/SKILL.md",
    "skills/src/assets/**/references/*.md",
    "core/src/tools/handlers/tool_*.txt",
    "core/src/agent/builtins/*.toml",
    "core/templates/**/*.xml",
    "apply-patch/*.md",
    "core/gpt*.md",
]

ROOT_LEVEL_FILES = [
    "core/prompt.md",
    "core/prompt_with_apply_patch_instructions.md",
    "core/review_prompt.md",
    "core/hierarchical_agents_message.md",
    "tui/prompt_for_init_command.md",
]


def discover_files(codex_root: str) -> dict[str, str]:
    """Discover files via glob patterns. Returns {relpath: absolute_path}."""
    discovered: dict[str, str] = {}

    for pattern in GLOB_PATTERNS:
        full_pattern = os.path.join(codex_root, pattern)
        for path in glob.glob(full_pattern, recursive=True):
            relpath = os.path.relpath(path, codex_root)
            discovered[relpath] = path

    for relpath in ROOT_LEVEL_FILES:
        abspath = os.path.join(codex_root, relpath)
        if os.path.isfile(abspath):
            discovered[relpath] = abspath

    return discovered


def discover_include_str(codex_root: str, already_discovered: set[str]) -> dict[str, str]:
    """Find include_str!("...") calls in .rs files and resolve paths.

    Only adds files not already in already_discovered.
    Filters to only .md, .txt, .xml, .toml extensions.
    """
    extra: dict[str, str] = {}
    try:
        result = subprocess.run(
            ["grep", "-rn", r'include_str!("', "--include=*.rs", "."],
            capture_output=True,
            text=True,
            cwd=codex_root,
            timeout=30,
        )
    except (subprocess.SubprocessError, OSError):
        return extra

    if result.returncode not in (0, 1):
        return extra

    include_re = re.compile(r'include_str!\(\s*"([^"]+)"\s*\)')
    valid_exts = {".md", ".txt", ".xml", ".toml"}
    # Exclude non-prompt files and test fixtures
    exclude_patterns = {
        "node-version.txt",
        "agent_names.txt",
        "tooltips.txt",
        "models.json",
        "third_party_models.json",
    }

    for line in result.stdout.splitlines():
        # line format: ./path/to/file.rs:NN:code
        parts = line.split(":", 2)
        if len(parts) < 3:
            continue
        rs_file = parts[0]
        code = parts[2]
        # Skip test files
        if "/tests/" in rs_file or "/test_" in rs_file or rs_file.endswith("_test.rs"):
            continue
        m = include_re.search(code)
        if not m:
            continue
        inc_path = m.group(1)
        # Skip concat! patterns (frame files etc.)
        if "concat!" in code:
            continue
        _, ext = os.path.splitext(inc_path)
        if ext not in valid_exts:
            continue
        basename = os.path.basename(inc_path)
        if basename in exclude_patterns:
            continue

        rs_dir = os.path.dirname(rs_file)
        resolved = os.path.normpath(os.path.join(codex_root, rs_dir, inc_path))
        if not os.path.isfile(resolved):
            continue
        relpath = os.path.relpath(resolved, codex_root)
        if relpath not in already_discovered:
            extra[relpath] = resolved

    return extra


# ---------------------------------------------------------------------------
# Entry assembly
# ---------------------------------------------------------------------------

def build_entry(
    name: str,
    category: str,
    source: str,
    content: str | None,
    git_meta: dict,
    description: str = "",
    origin: str = "auto-discovered",
    include_content: bool = True,
) -> dict:
    """Build a single entry dict."""
    text = content or ""
    tokens = token_count(text) if text else 0
    byte_count = len(text.encode("utf-8")) if text else 0

    entry: dict = {
        "name": name,
        "category": category,
        "source": source,
        "tokens": tokens,
        "bytes": byte_count,
        "git_modified": git_meta.get("modified", ""),
        "git_author": git_meta.get("author", ""),
        "git_date": git_meta.get("date", ""),
        "description": description,
        "origin": origin,
    }
    if include_content and content is not None:
        entry["content"] = content
    return entry


def process_registry_entries(
    registry_path: str,
    codex_root: str,
    metadata_only: bool,
    git_metas: dict[str, dict],
) -> list[dict]:
    """Process entries from prompt-registry.toml."""
    with open(registry_path, "rb") as f:
        registry = tomllib.load(f)

    entries: list[dict] = []
    for item in registry.get("entries", []):
        name = item.get("name", "")
        category_slug = item.get("category", "")
        category = CATEGORY_SLUG_MAP.get(category_slug, category_slug)
        filepath = item.get("file", "")
        pattern = item.get("pattern", "")
        entry_type = item.get("type", "")
        description = item.get("description", "")
        abspath = os.path.join(codex_root, filepath)

        content = None
        line_number = 0

        # Always extract content (needed for token counting even in metadata-only mode)
        if entry_type == "function":
            content, line_number = extract_rust_function(abspath, pattern)
        else:
            content, line_number = extract_rust_const(abspath, pattern)

        source = f"{filepath}:{line_number}" if line_number else filepath
        git_meta = git_metas.get(filepath, {})

        entries.append(build_entry(
            name=name,
            category=category,
            source=source,
            content=content,
            git_meta=git_meta,
            description=description,
            origin="registry",
            include_content=not metadata_only,
        ))

    return entries


def process_discovered_files(
    discovered: dict[str, str],
    codex_root: str,
    metadata_only: bool,
    git_metas: dict[str, dict],
) -> list[dict]:
    """Process auto-discovered files."""
    entries: list[dict] = []
    for relpath, abspath in sorted(discovered.items()):
        name = _name_from_path(relpath)
        category = _categorize_path(relpath)

        # Always extract content (needed for token counting even in metadata-only mode)
        content = None
        try:
            content = extract_file_content(abspath)
        except OSError:
            content = None

        git_meta = git_metas.get(relpath, {})
        source = f"{relpath}:1"

        entries.append(build_entry(
            name=name,
            category=category,
            source=source,
            content=content,
            git_meta=git_meta,
            description="",
            origin="auto-discovered",
            include_content=not metadata_only,
        ))

    return entries


def extract_skill_descriptions(
    discovered: dict[str, str],
    codex_root: str,
    metadata_only: bool,
    git_metas: dict[str, dict],
) -> list[dict]:
    """Extract name + description frontmatter from SKILL.md files.

    These are the short descriptions always present in the agent's context
    via render_skills_section(), separate from the full SKILL.md content.
    """
    entries: list[dict] = []
    for relpath, abspath in sorted(discovered.items()):
        if not relpath.endswith("SKILL.md"):
            continue
        try:
            raw = extract_file_content(abspath)
        except OSError:
            continue
        if not raw:
            continue
        # Parse YAML frontmatter
        if not raw.startswith("---"):
            continue
        end = raw.find("---", 3)
        if end < 0:
            continue
        frontmatter = raw[3:end].strip()
        name = ""
        description = ""
        lines = frontmatter.splitlines()
        i = 0
        while i < len(lines):
            line = lines[i]
            if line.startswith("name:"):
                name = line[len("name:"):].strip().strip("'\"")
            elif line.startswith("description:"):
                value = line[len("description:"):].strip()
                if value in (">-", ">", "|", "|-"):
                    # Multi-line YAML scalar — collect indented continuation lines
                    parts = []
                    i += 1
                    while i < len(lines) and (lines[i].startswith("  ") or lines[i].startswith("\t")):
                        parts.append(lines[i].strip())
                        i += 1
                    description = " ".join(parts)
                    continue  # skip the i += 1 at the bottom
                else:
                    description = value.strip("'\"")
            i += 1
        if not name:
            continue
        # The "content" for token counting is what render_skills_section outputs per skill:
        # roughly "- name: description"
        listing_text = f"- {name}: {description}"
        git_meta = git_metas.get(relpath, {})

        entries.append(build_entry(
            name=name,
            category="Skill Descriptions",
            source=f"{relpath}:1",
            content=listing_text,
            git_meta=git_meta,
            description=description,
            origin="auto-discovered",
            include_content=not metadata_only,
        ))
    return entries


def group_by_category(entries: list[dict]) -> list[dict]:
    """Group entries by category and compute totals."""
    categories: dict[str, list[dict]] = {}
    for entry in entries:
        cat = entry.get("category", "Uncategorized")
        categories.setdefault(cat, []).append(entry)

    result = []
    for cat_name in sorted(categories.keys()):
        cat_entries = categories[cat_name]
        total_tokens = sum(e.get("tokens", 0) for e in cat_entries)
        result.append({
            "name": cat_name,
            "total_tokens": total_tokens,
            "entries": cat_entries,
        })
    return result


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(description="Prompt Inspector data generator")
    parser.add_argument("--codex-root", required=True, help="Path to codex-rs/")
    parser.add_argument("--metadata-only", action="store_true",
                        help="Output JSON without content fields")
    parser.add_argument("--entry", type=str, default=None,
                        help="Output JSON for a single entry with full content")
    args = parser.parse_args()

    codex_root = os.path.abspath(args.codex_root)

    # 1. Load registry
    registry_path = os.path.join(codex_root, "tools", "prompt-inspector", "prompt-registry.toml")
    if not os.path.isfile(registry_path):
        print(f"Error: registry not found at {registry_path}", file=sys.stderr)
        sys.exit(1)

    # 2. Auto-discover files
    discovered = discover_files(codex_root)

    # 3. Find include_str! references
    include_str_files = discover_include_str(codex_root, set(discovered.keys()))
    discovered.update(include_str_files)

    # 4. Collect all file paths for batch git metadata
    with open(registry_path, "rb") as f:
        registry_data = tomllib.load(f)
    registry_filepaths = [e.get("file", "") for e in registry_data.get("entries", [])]
    all_filepaths = list(set(registry_filepaths + list(discovered.keys())))

    # 5. Batch git metadata
    git_metas = git_metadata_batch(all_filepaths, codex_root)

    # 6. Process registry entries (always include, even with --entry)
    registry_entries = process_registry_entries(
        registry_path, codex_root,
        metadata_only=args.metadata_only and args.entry is None,
        git_metas=git_metas,
    )

    # 7. Process discovered files
    file_entries = process_discovered_files(
        discovered, codex_root,
        metadata_only=args.metadata_only and args.entry is None,
        git_metas=git_metas,
    )

    # 8. Extract skill descriptions (always-in-context frontmatter)
    skill_desc_entries = extract_skill_descriptions(
        discovered, codex_root,
        metadata_only=args.metadata_only and args.entry is None,
        git_metas=git_metas,
    )

    all_entries = registry_entries + file_entries + skill_desc_entries

    # 8. Handle --entry: find matching entry, output just that
    if args.entry is not None:
        search = args.entry.lower()
        match = None
        for entry in all_entries:
            if entry["name"].lower() == search:
                match = entry
                break
        if match is None:
            # Try partial match
            for entry in all_entries:
                if search in entry["name"].lower():
                    match = entry
                    break
        if match is None:
            print(json.dumps({"error": f"Entry '{args.entry}' not found"}))
            sys.exit(1)

        # Ensure content is populated for --entry
        if "content" not in match or match.get("content") is None:
            # Re-extract content
            if match.get("origin") == "registry":
                for item in registry_data.get("entries", []):
                    if item.get("name") == match["name"]:
                        abspath = os.path.join(codex_root, item.get("file", ""))
                        pattern = item.get("pattern", "")
                        if item.get("type") == "function":
                            content, _ = extract_rust_function(abspath, pattern)
                        else:
                            content, _ = extract_rust_const(abspath, pattern)
                        if content is not None:
                            match["content"] = content
                            match["tokens"] = token_count(content)
                            match["bytes"] = len(content.encode("utf-8"))
                        break
            else:
                source = match.get("source", "")
                relpath = source.split(":")[0] if ":" in source else source
                abspath = os.path.join(codex_root, relpath)
                try:
                    content = extract_file_content(abspath)
                    match["content"] = content
                    match["tokens"] = token_count(content)
                    match["bytes"] = len(content.encode("utf-8"))
                except OSError:
                    pass

        print(json.dumps(match, indent=2))
        return

    # 9. Group by category and build output
    categories = group_by_category(all_entries)
    total_tokens = sum(c["total_tokens"] for c in categories)

    output = {
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "total_tokens": total_tokens,
        "categories": categories,
    }

    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
