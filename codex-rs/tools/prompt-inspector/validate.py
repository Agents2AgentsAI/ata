#!/usr/bin/env python3
"""Prompt Inspector — drift detection script.

Checks that the prompt registry stays in sync with the codebase.

Usage:
    python3 validate.py [--codex-root PATH]
"""

from __future__ import annotations

import argparse
import glob
import os
import re
import subprocess
import sys

# Try tomllib (3.11+) then tomli
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore[no-redef]


# ---------------------------------------------------------------------------
# Auto-discovery glob patterns (must match generate.py)
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
    # v0.129.0+ paths: agent-facing templates that moved out of core/templates.
    "core/src/context/prompts/**/*.md",
    "core/src/guardian/*.md",
    "collaboration-mode-templates/templates/*.md",
    "memories/write/templates/**/*.md",
    "memories/read/templates/**/*.md",
]

ROOT_LEVEL_FILES = [
    "core/prompt.md",
    "core/prompt_with_apply_patch_instructions.md",
    "core/review_prompt.md",
    "core/hierarchical_agents_message.md",
    "tui/prompt_for_init_command.md",
    # v0.129.0+ moved the personality base prompt into the models-manager crate.
    "models-manager/prompt.md",
]

# Substrings that indicate a test-only include — skip these
TEST_SKIP_SUBSTRINGS = ["test", "fixture", "snapshot"]

# Extensions considered potentially agent-facing (mirrors generate.py)
VALID_EXTENSIONS = {".md", ".txt", ".xml", ".toml"}

# Basenames explicitly excluded by generate.py's discover_include_str
EXCLUDED_BASENAMES = {
    "node-version.txt",
    "agent_names.txt",
    "tooltips.txt",
    "models.json",
    "third_party_models.json",
}


def _matches_any_glob(relpath: str, codex_root: str) -> bool:
    """Return True if relpath matches any auto-discovery glob pattern or root-level file list."""
    # Normalise to forward-slash
    relpath = relpath.replace(os.sep, "/")

    # Check explicit root-level files
    if relpath in ROOT_LEVEL_FILES:
        return True

    # Check glob patterns (fnmatch with ** support via glob.glob)
    for pattern in GLOB_PATTERNS:
        # Use fnmatch-style matching; handle ** by expanding the glob against the filesystem
        # and then checking membership.
        full_pattern = os.path.join(codex_root, pattern)
        for matched in glob.glob(full_pattern, recursive=True):
            matched_rel = os.path.relpath(matched, codex_root).replace(os.sep, "/")
            if matched_rel == relpath:
                return True

    return False


# ---------------------------------------------------------------------------
# Check 1 — include_str! coverage
# ---------------------------------------------------------------------------

def check_include_str_coverage(codex_root: str) -> list[str]:
    """Warn if any include_str!("...") target isn't covered by auto-discovery globs."""
    warnings: list[str] = []

    try:
        result = subprocess.run(
            ["grep", "-rn", r'include_str!("', "--include=*.rs", "."],
            capture_output=True,
            text=True,
            cwd=codex_root,
            timeout=120,
        )
    except (subprocess.SubprocessError, OSError) as exc:
        warnings.append(f"  [warn] Could not run grep for include_str!: {exc}")
        return warnings

    if result.returncode not in (0, 1):
        warnings.append("  [warn] grep failed for include_str! scan")
        return warnings

    include_re = re.compile(r'include_str!\(\s*"([^"]+)"\s*\)')

    for line in result.stdout.splitlines():
        parts = line.split(":", 2)
        if len(parts) < 3:
            continue
        rs_file = parts[0]
        code = parts[2]

        # Skip test files
        rs_lower = rs_file.lower()
        if any(skip in rs_lower for skip in TEST_SKIP_SUBSTRINGS):
            continue

        m = include_re.search(code)
        if not m:
            continue

        inc_path = m.group(1)

        # Only consider extensions that are potentially agent-facing
        _, ext = os.path.splitext(inc_path)
        if ext not in VALID_EXTENSIONS:
            continue

        # Skip basenames explicitly excluded by generate.py
        basename = os.path.basename(inc_path)
        if basename in EXCLUDED_BASENAMES:
            continue

        # Resolve the path relative to the .rs file's directory
        # rs_file is like ./path/to/file.rs (relative to codex_root)
        rs_dir = os.path.dirname(os.path.join(codex_root, rs_file.lstrip("./")))
        resolved = os.path.normpath(os.path.join(rs_dir, inc_path))
        if not os.path.isfile(resolved):
            continue

        relpath = os.path.relpath(resolved, codex_root).replace(os.sep, "/")

        # Skip paths that contain test/fixture/snapshot indicators
        if any(skip in relpath.lower() for skip in TEST_SKIP_SUBSTRINGS):
            continue

        if not _matches_any_glob(relpath, codex_root):
            warnings.append(
                f"  [warn] include_str! target not covered by auto-discovery: {relpath}"
                f"  (referenced from {rs_file.lstrip('./')})"
            )

    return warnings


# ---------------------------------------------------------------------------
# Check 2 — @agent-facing coverage
# ---------------------------------------------------------------------------

def check_agent_facing_coverage(codex_root: str, registry_entries: list[dict]) -> list[str]:
    """Warn if any // @agent-facing annotation has no matching registry entry."""
    warnings: list[str] = []

    try:
        result = subprocess.run(
            ["grep", "-rn", r"// @agent-facing", "--include=*.rs", "."],
            capture_output=True,
            text=True,
            cwd=codex_root,
            timeout=120,
        )
    except (subprocess.SubprocessError, OSError) as exc:
        warnings.append(f"  [warn] Could not scan for @agent-facing annotations: {exc}")
        return warnings

    if result.returncode == 1:
        # No matches — fine
        return warnings
    if result.returncode != 0:
        warnings.append("  [warn] grep failed for @agent-facing scan")
        return warnings

    # Build a lookup: (relpath, pattern) -> True
    registry_lookup: set[tuple[str, str]] = set()
    for entry in registry_entries:
        file_rel = entry.get("file", "").replace(os.sep, "/")
        pattern = entry.get("pattern", "")
        if file_rel and pattern:
            registry_lookup.add((file_rel, pattern))

    # Also build a pattern-only lookup for fallback matching
    registry_patterns: set[str] = {entry.get("pattern", "") for entry in registry_entries}

    annotation_re = re.compile(r"//\s*@agent-facing")
    # Matches const/fn/static name on a non-comment, non-blank line. Anchored
    # on the item keyword (`fn`/`const`/`static`) and captures the very next
    # identifier; this avoids the previous regex picking up a type token like
    # `LazyLock` or `str` from `: LazyLock<...> = ...` / `: &str = ...`.
    ident_re = re.compile(r"\b(?:fn|const|static)\b\s+([A-Za-z_][A-Za-z0-9_]*)")

    # Collect annotations: (rs_file_relpath, line_number, matched_name_or_None)
    annotated_lines: list[tuple[str, int, str]] = []

    lines_by_file: dict[str, list[tuple[int, str]]] = {}
    for line in result.stdout.splitlines():
        parts = line.split(":", 2)
        if len(parts) < 3:
            continue
        rs_file = parts[0].lstrip("./")
        try:
            lineno = int(parts[1])
        except ValueError:
            continue
        lines_by_file.setdefault(rs_file, []).append((lineno, parts[2]))

    for rs_file, annotations in lines_by_file.items():
        abs_rs = os.path.join(codex_root, rs_file)
        try:
            with open(abs_rs, encoding="utf-8") as f:
                all_lines = f.readlines()
        except OSError:
            for lineno, _ in annotations:
                annotated_lines.append((rs_file, lineno, ""))
            continue

        for lineno, _ in annotations:
            # Find the next non-comment, non-empty, non-attribute line after
            # the annotation. `#[allow(dead_code)]` style attribute lines
            # frequently sit between the // @agent-facing comment and the
            # actual item, so skip them too instead of treating them as the
            # target.
            found_name = ""
            for i in range(lineno, min(lineno + 10, len(all_lines))):
                candidate = all_lines[i].strip()
                if (
                    not candidate
                    or candidate.startswith("//")
                    or candidate.startswith("#[")
                ):
                    continue
                m = ident_re.search(candidate)
                if m:
                    found_name = m.group(1)
                break
            annotated_lines.append((rs_file, lineno, found_name))

    for rs_file, lineno, name in annotated_lines:
        if not name:
            warnings.append(
                f"  [warn] @agent-facing annotation at {rs_file}:{lineno} — could not determine identifier name"
            )
            continue

        rs_rel = rs_file.replace(os.sep, "/")
        if (rs_rel, name) not in registry_lookup and name not in registry_patterns:
            warnings.append(
                f"  [warn] @agent-facing annotation not in registry: {name}"
                f"  ({rs_rel}:{lineno})"
            )

    return warnings


# ---------------------------------------------------------------------------
# Check 3 — Registry integrity
# ---------------------------------------------------------------------------

def check_registry_integrity(codex_root: str, registry_entries: list[dict]) -> list[str]:
    """Error if any registry entry points to a missing file or unfound pattern."""
    errors: list[str] = []

    for entry in registry_entries:
        name = entry.get("name", "<unnamed>")
        rel_file = entry.get("file", "")
        pattern = entry.get("pattern", "")

        if not rel_file:
            errors.append(f"  [error] Registry entry '{name}' has no 'file' field")
            continue

        abs_file = os.path.join(codex_root, rel_file)
        if not os.path.isfile(abs_file):
            errors.append(
                f"  [error] Registry entry '{name}': file not found: {rel_file}"
            )
            continue

        if not pattern:
            errors.append(f"  [error] Registry entry '{name}' has no 'pattern' field")
            continue

        # Grep the file for the pattern string
        try:
            with open(abs_file, encoding="utf-8") as f:
                content = f.read()
        except OSError as exc:
            errors.append(f"  [error] Registry entry '{name}': could not read {rel_file}: {exc}")
            continue

        if pattern not in content:
            errors.append(
                f"  [error] Registry entry '{name}': pattern '{pattern}' not found in {rel_file}"
            )

    return errors


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def _auto_detect_codex_root() -> str:
    """Detect codex-rs root from this script's location (../../ from script dir)."""
    script_dir = os.path.dirname(os.path.abspath(__file__))
    # tools/prompt-inspector/validate.py -> two levels up = codex-rs/
    return os.path.normpath(os.path.join(script_dir, "..", ".."))


def main() -> None:
    parser = argparse.ArgumentParser(description="Prompt Inspector drift detection")
    parser.add_argument(
        "--codex-root",
        default=None,
        help="Path to codex-rs/ (default: auto-detect from script location)",
    )
    args = parser.parse_args()

    codex_root = os.path.abspath(args.codex_root) if args.codex_root else _auto_detect_codex_root()

    registry_path = os.path.join(codex_root, "tools", "prompt-inspector", "prompt-registry.toml")
    if not os.path.isfile(registry_path):
        print(f"[error] Registry not found at: {registry_path}", file=sys.stderr)
        sys.exit(1)

    with open(registry_path, "rb") as f:
        registry_data = tomllib.load(f)
    registry_entries = registry_data.get("entries", [])

    print("Prompt Inspector — drift detection")
    print(f"  codex-root : {codex_root}")
    print(f"  registry   : {registry_path}")
    print(f"  entries    : {len(registry_entries)}")
    print()

    all_warnings: list[str] = []
    all_errors: list[str] = []

    # --- Check 1 ---
    print("Check 1: include_str! coverage")
    c1 = check_include_str_coverage(codex_root)
    if c1:
        all_warnings.extend(c1)
        for msg in c1:
            print(msg)
    else:
        print("  OK — all include_str! targets are covered by auto-discovery globs")
    print()

    # --- Check 2 ---
    print("Check 2: @agent-facing annotation coverage")
    c2 = check_agent_facing_coverage(codex_root, registry_entries)
    if c2:
        all_warnings.extend(c2)
        for msg in c2:
            print(msg)
    else:
        print("  OK — no unregistered @agent-facing annotations found")
    print()

    # --- Check 3 ---
    print("Check 3: registry integrity")
    c3 = check_registry_integrity(codex_root, registry_entries)
    if c3:
        all_errors.extend(c3)
        for msg in c3:
            print(msg)
    else:
        print("  OK — all registry entries point to valid files with matching patterns")
    print()

    # --- Summary ---
    total_warnings = len(all_warnings)
    total_errors = len(all_errors)

    if total_warnings == 0 and total_errors == 0:
        print("Result: CLEAN — no issues found.")
        sys.exit(0)
    else:
        print(
            f"Result: {total_errors} error(s), {total_warnings} warning(s)."
        )
        if total_errors > 0:
            sys.exit(1)
        # Warnings-only: still exit 0 (warnings don't block CI)
        sys.exit(0)


if __name__ == "__main__":
    main()
