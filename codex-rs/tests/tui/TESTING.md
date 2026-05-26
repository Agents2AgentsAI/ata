# ATA TUI testing guide

This is the full guide to how the ATA terminal app is tested. If you only have one minute: open `PLAN.md` to see what we test, look at `run-tier-b1.sh` to see how a test is written, then check `.github/workflows/tui-tests-tier-b1.yml` to see how CI runs it.


## What this is

ATA is a terminal app. You type, it shows menus, lists, readers, and chat. We want to make sure that when we change the code, none of those screens break.

So we wrote scripts that drive the app the way a real user would: type keys, look at the screen, and check that the result is what we expect. Those scripts run automatically on every pull request through GitHub Actions, so we catch breakage before it lands on the main branch.

There are three layers:

1. `PLAN.md` is the human-readable list of test cases. Each one is called a TR (Test Requirement). Some TRs have just one test; others have several variants called "scenarios" labeled A, B, C and so on.
2. The runner scripts (`run-tier-a.sh`, `run-tier-b1.sh`, `run-tier-b2.sh`) are the actual tests. Each TR or scenario in `PLAN.md` becomes one shell function inside a runner.
3. The GitHub Actions workflows (`.github/workflows/tui-tests-tier-*.yml`) run the scripts on every pull request (or only when a human clicks a button, depending on the tier).

There's also `README.md` next to this file. It explains the older agent-driven flow where you ask Claude with the `ata-tmux-test` skill to run `PLAN.md` and write a report. That flow still works for one-off manual runs. The runners and CI in this guide are the automated version of the same idea.


## The three tiers in plain English

We split tests by what they need to run. The split matters because some tests cost money (real model calls) and shouldn't fire on every push.

### Tier A — command line only

- **File:** `run-tier-a.sh`
- **Workflow:** `.github/workflows/tui-tests-tier-a.yml`
- **Runs on:** every pull request, automatically
- **Cost:** free (no model calls)

These tests use ATA's command line interface like `ata workspace add ...` or `ata hooks list ...`. They don't open the TUI and don't talk to a language model. They are the cheapest tests and finish in under a minute.

Examples of what's covered: workspace management (TR-055), hook commands (TR-056), skill management (TR-057), config edits (TR-058 through TR-061).

### Tier B1 — TUI without a model

- **File:** `run-tier-b1.sh`
- **Workflow:** `.github/workflows/tui-tests-tier-b1.yml`
- **Runs on:** every pull request, automatically
- **Cost:** free (no model calls)

These tests open the real TUI inside a `tmux` session, type keys into it, and check what shows up on screen. They use a fake API key, so any time the model would be called the call fails. That's fine for testing menus, slash commands, the file mention picker, history recall, plan mode, panel layouts, and anything else that doesn't depend on a model reply.

Run time: roughly 7-8 minutes total in CI.

Examples of what's covered: startup (TR-003), slash menu (TR-004), `/clear` (TR-016), permissions panel (TR-017), model picker (TR-018), file mention picker (TR-019), workspace commands (TR-040), subagents picker (TR-041), plan mode (TR-043), `/resume` picker (TR-046), and many more. About 58 scenarios.

### Tier B2 — TUI with a real model

- **File:** `run-tier-b2.sh`
- **Workflow:** `.github/workflows/tui-tests-tier-b2.yml`
- **Runs on:** only when a human clicks "Run workflow" in the GitHub Actions tab. Never automatic.
- **Cost:** real money (each test sends real prompts to a real model)

These tests need the model to actually reply. We use them for things like the reading view (opened by asking the model to make a document), the `/fork` and `/compact` slash commands (which need a real conversation), and tool routing (where we check the model picked the right tool).

To keep the bill from running on every push, this tier ONLY fires when someone clicks the "Run workflow" button. There's no auto-trigger.

The workflow accepts any one of these provider keys as a repo secret:

| Secret name           | Provider  | Default model         |
|-----------------------|-----------|-----------------------|
| `OPENAI_API_KEY`      | openai    | gpt-5.3-codex         |
| `ANTHROPIC_API_KEY`   | anthropic | claude-sonnet-4-6     |
| `GOOGLE_API_KEY`      | gemini    | gemini-3.1-pro-preview|

When more than one is set the workflow picks openai first, then anthropic, then gemini. The tests were written against gpt-5.5, so the content checks should hold across providers. Tool-routing checks may behave differently on Anthropic or Gemini because each model has its own routing preferences.

About 63 scenarios.


## What `PLAN.md` is

`PLAN.md` is the source of truth for what we test and why. Each TR section looks like this:

```
## TR-006: Up-arrow history

Setup: TR-005 (submit at least one message first).

### Scenario A: baseline - Up recalls last submission
1. C-u, then Up
2. capture as `up`.

Expect:
- `up` contains `respond with just hi` in the composer
```

The `Expect` block is the contract. The runner script translates that into a shell assertion:

```bash
assert_contains "$out" "respond with just hi" "Up recalls last submission"
```

Rule of thumb: if you change `PLAN.md`, change the runner. If you change the runner, update `PLAN.md`. They should stay in sync.


## How a test is built

Every test function in the runner follows the same shape:

```bash
tr006_b() {
  start_test "TR-006 B"                      # 1. announce the test
  local sess=$SESSION-006b                   # 2. pick a unique tmux session name
  if ! boot_ata "$sess"; then                # 3. start ATA, wait for the banner
    fail_assert "ata never reached the composer"
    end_test; kill_ata "$sess"; return
  fi
  send_text "$sess" "/clear"                 # 4. drive the UI like a user would
  send_key  "$sess" Enter
  sleep 2
  local out=$WORK/006b.txt                   # 5. capture the visible pane
  capture "$sess" "$out"
  assert_contains "$out" "Token usage:"      # 6. check the result with assertions
  kill_ata "$sess"                           # 7. clean up
  end_test
}
```

The naming convention is `trXXX_y` where `XXX` is the TR number (zero-padded to three digits) and `y` is the scenario letter (lowercase).

Then add the function name to the driver near the bottom of the file:

```bash
log "Numbered TRs (in order)"
tr006_a; tr006_b; tr006_c
```

Tests inside a TR are alphabetized; TRs themselves are listed in numeric order.


## Helper functions you'll see

Defined near the top of each runner.

| Helper | What it does |
|---|---|
| `boot_ata "name"` | Start ATA in a fresh tmux session, wait until the banner shows AND no task is in flight. Returns failure if it doesn't settle within 60 seconds. |
| `kill_ata "name"` | Kill the tmux session by name. |
| `send_text "name" "..."` | Type some text into the session (no Enter pressed). |
| `send_key "name" Enter` | Press a single key. Names like `Enter`, `Escape`, `Tab`, `BTab` (Shift+Tab), `C-u`, `C-c`, `M-Left` are all accepted. |
| `capture "name" file.txt` | Save the current visible pane to a file. |
| `wait_for_idle "name" 60` (B2) | Poll for the "esc to interrupt" indicator to disappear, meaning the model finished replying. |
| `assert_contains file "needle" "description"` | Pass if the file has the needle string. Fail otherwise. |
| `assert_not_contains file "needle" "desc"` | Pass if the file does NOT have the needle. |
| `assert_match file "regex" "desc"` | Pass if the file matches the regex. Use this when "contains" isn't precise enough. |
| `fail_assert "reason"` | Mark the test failed and dump the last 800 bytes of the pane. |
| `skip_test "reason"` | Mark the test skipped (for cases like "needs debug build" or "needs Linux"). |
| `start_test "TR-XXX Y"` | Announce the start of a test for the output log. |
| `end_test` | Mark the test done (pass if no asserts failed). |

There's also `boot_reader` in `run-tier-b2.sh`. It calls `boot_ata`, then asks the model to "give me 2 short slides on coffee in reading view", and waits for the reading view to open. Once it returns, the reader is ready and your test can press keys into it.


## How to run the tests locally

You need `tmux`, `bash`, and the local debug build of ATA.

```sh
# Build ATA in debug mode (release works for most tests but not for /rollout):
cd codex-rs
cargo build -p codex-cli --bin ata

# Tier A:
bash tests/tui/run-tier-a.sh

# Tier B1 (uses your real ~/.ata config and history):
bash tests/tui/run-tier-b1.sh

# Tier B2 (real model calls, costs money):
bash tests/tui/run-tier-b2.sh
```

The default `ATA_BIN` is `ata` from your `PATH`. To point at a specific binary:

```sh
ATA_BIN=/path/to/target/debug/ata bash tests/tui/run-tier-b1.sh
```

The output for each scenario looks like one of these:

```
  TR-006 A       PASS
  TR-006 B       SKIP — needs debug build
  TR-006 C       FAIL
    [TR-006 C] Down moves forward to middle entry
      got: <last 800 bytes of the captured pane>
```

The script ends with a summary line like `PASS: 56  FAIL: 0  SKIP: 2`. Exit code is `0` if everything passed or skipped, `1` if any test failed.

To debug a specific failure, redirect to a file and inspect:

```sh
bash tests/tui/run-tier-b1.sh > /tmp/run.log 2>&1
grep -B 2 -A 10 "TR-006 C" /tmp/run.log
```

If you want to interactively reproduce a TUI behavior without running the whole runner, the `ata-tmux-test` skill (in `~/.claude/skills/`) gives Claude the recipe for driving ATA in tmux. Useful for "what does the pane actually look like when I press X".


## How CI runs the tests

CI is GitHub Actions. The three workflows live in `.github/workflows/`:

- `tui-tests-tier-a.yml` — runs on every pull request, automatically
- `tui-tests-tier-b1.yml` — runs on every pull request, automatically
- `tui-tests-tier-b2.yml` — runs only when a human clicks "Run workflow" in the Actions tab

Each workflow does roughly:

1. Check out the code.
2. Install the Rust toolchain (cache cargo across runs).
3. Build the ATA binary with `cargo build -p codex-cli --bin ata` (debug build).
4. Install `tmux` (only needed for B1 and B2).
5. Write a fake `~/.ata/auth.json` and a pre-trusted `~/.ata/config.toml` so ATA doesn't show a login screen or a "trust this folder?" prompt.
6. Run the matching `run-tier-*.sh` script.
7. Mark the job green if all scenarios pass or skip; red if any failed.

The fake auth in B1 is enough because B1 doesn't actually use the model. B2 swaps it for the real provider key from the repo secret.

To trigger a B2 run manually:

1. Go to https://github.com/Tim1406/ata/actions/workflows/tui-tests-tier-b2.yml
2. Click "Run workflow" on the right
3. Pick the branch
4. Click the green "Run workflow" button

It costs money. Each scenario sends real prompts.


## What we don't test (and why)

| TR(s) | Why not in CI |
|---|---|
| TR-012, TR-013 (voice mode) | `/voice` is compiled out on Linux (`#[cfg(not(target_os = "linux"))]`). Ubuntu CI doesn't have the command. Tests skip on Linux and run on macOS. |
| TR-015 | PLAN.md marks it "superseded by TR-042". Nothing to test. |
| TR-024 through TR-030 (scheduling lifecycle) | These cross the real cron daemon, the file system, and subprocess streams on wall-clock timers. A 70-second wait for a system cron to fire is too flaky on a shared CI runner. |
| TR-028 (monitor_watch_for) | Verifies the model picks the right tool between `monitor_wait` and `monitor_watch_for`. Tool routing is non-deterministic. No honest workaround without statistical sampling (run N times, alert if win-rate drops). |
| TR-035 (multi-source synthesis) | Polls up to 10 minutes for the model to use both Hacker News and paper search. Slow and weak signal — the model often spawns sub-agents, so its tool calls live in different session files. |
| TR-038 B, E, E2, F (`/copy` edge cases) | Scenarios A, C, and D are now covered (xclip + Xvfb installed in CI). B needs nuanced multi-line list assertions; E and E2 need `/side` priming flows; F needs the in-flight + completed-turn precondition. Open work for a follow-up. |
| TR-042 A (release-build `/rollout`) | The command is hidden in release builds. CI always builds debug, so the negative case is untestable. |
| TR-044 C (`/side` recursion guard) | The unit test in `chatwidget/tests/side.rs` confirms the guard works, but in a live tmux session the expected error doesn't surface on screen. Needs deeper investigation. Marked SKIP for now. |
| TR-048 (`/goal`) | Requires the `Feature::Goals` build flag, which isn't on by default. |
| TR-061 D through L (Zotero GUI) | These flows need a real Zotero install responding to real API calls. Out of scope for headless CI. |
| TR-064, TR-065, TR-066 (`paper_*` tools) | The dedicated tools only fire when the prompt names them explicitly (e.g. "use the paper_citations tool"). That's "cheating" — it tests the tool runs, not that the model picks it. Without explicit naming the model falls back to `exec_command` curl. No honest workaround. |
| Various "during in-flight turn" scenarios (TR-010 D, TR-016 C, TR-017 E, TR-018 D, TR-040 G, TR-046 E) | Need to start a slow model call, then test that a slash command is blocked while the call is mid-flight. B1 has no real model. The B2 versions are open work. |

If you want to attempt one of the skipped ones later: write it as a B2 scenario, pick a deterministic signal you can wait on (a specific log line, a file change, a JSONL event), and accept that the test will probably need to retry a few times before it's stable enough to merge.


## Known divergences between `PLAN.md` and ATA today

These are places where `PLAN.md` and current ATA disagree. The runner picks whichever interpretation actually works on current ATA and leaves a comment pointing back to the divergence.

| TR | PLAN.md says | Reality on ATA 0.7.0 | What the test does |
|---|---|---|---|
| TR-019 E | Escape leaves the `@`-picker UI visible | Escape clears the picker contents; only the `@xyz` text remains in the composer | Asserts the composer text remains; doesn't assert the picker UI is still visible |
| TR-046 B | Rows show `<N> ago` (long form like "3 days ago") | Rows show `<N>d ago` (compact form, like "3d ago") | Uses the compact-form regex |
| TR-054 D | `?` toggles the help overlay closed | Second `?` doesn't close; only Escape does | Probes for the toggle, falls back to Escape if needed, prints a yellow note about the divergence |

When ATA changes to match `PLAN.md` (or vice versa), update both files in the same PR.


## Conventions worth knowing

**Why we poll instead of just `sleep`.** CI is slower than a laptop. A fixed `sleep 2` can race against a slow redraw. We poll until the pane shows a known sentinel string (like "Agents2Agents ata" for the banner or "Sections (n/p" for the reader). The test waits exactly as long as needed and no more.

**Why content-level asserts, not exact-string asserts.** Models are non-deterministic. We can't say "the response is exactly X." But we can say "the response contains 'Asynchronous Rust'" or "matches `[0-9]+ points, [0-9]+ comments`." Pick predicates that are hard to fake but loose enough to survive normal variation.

**Why a fresh `~/.ata/` in CI sometimes breaks tests.** Tests that assume "I have prior sessions" or "there's already a workspace set up" will fail on a cold runner. Either seed the state inside the test (write to `history.jsonl`, send a chat first), or use predicates that gracefully handle the empty case.

**Why some predicates use regex and not literal strings.** Linux and macOS sometimes return files in different orders, and certain widgets render slightly differently. For example, `@Cargo` Tab on Mac picks `Cargo.toml` first; on Linux CI it picks `Cargo.lock`. Both are valid path-injection (the test's point). So the predicate uses `Cargo\.(toml|lock)` instead of either literal.

**Why each test creates its own tmux session.** So tests don't share state. If one test sets a model and the next one assumes the default model, you've got a problem. Each session boots fresh.

**Why we sometimes use a "soft" tool-call check.** Tool routing is non-deterministic. For TR-011 (code_intel) and TR-063 A (paper_get), the test passes if the *answer* is correct, and prints a yellow note if the *expected* tool wasn't called. This matches PLAN.md's own fallback clause.


## Quick lookup table

| Question | Answer |
|---|---|
| Are CI tests automatic on every PR? | Yes for A and B1. No for B2 (manual only). |
| Do CI tests cost money? | A and B1 are free. B2 costs real model tokens per run. |
| Where is the list of what we test? | `PLAN.md`. |
| Where is the actual test code? | `run-tier-a.sh`, `run-tier-b1.sh`, `run-tier-b2.sh`. |
| Where is the CI config? | `.github/workflows/tui-tests-tier-*.yml`. |
| How do I run them locally? | `bash run-tier-XX.sh` (debug build of ATA needed). |
| How do I trigger B2 manually? | Actions tab → tui-tests-tier-b2 → Run workflow. |
| Why did one of my tests pass on Mac but fail in CI? | Probably platform-specific file ordering, timing, or a missing tool (xclip, audio device). |
| Why does my new test work locally but fail on a fresh CI runner? | A fresh `~/.ata/` doesn't have the prior state your test assumed (workspace, sessions). Seed it in the test. |


## Glossary

- **TR**: Test Requirement. One numbered entry in `PLAN.md`.
- **Scenario**: A variant of a TR. Most TRs have a baseline (Scenario A) and may have alternatives (B, C, and so on).
- **Tier A / B1 / B2**: The three categories of tests, sorted from cheapest (A) to most expensive (B2).
- **Smoke**: A quick sanity run of the tests. The term comes from "if smoke comes out when you turn it on, it's broken." We use it to catch obvious mistakes (syntax errors, broken predicates) before doing deeper verification.
- **Predicate**: A condition the test checks (`assert_contains`, `assert_match`, etc.).
- **Sentinel**: A specific string the test polls for to know when ATA is in the state we want (e.g. "Agents2Agents ata" for boot, "q: close" for the reader being open).
- **`boot_ata` / `boot_reader`**: Helpers that start ATA in a fresh tmux session and wait until it's ready for input.
- **JSONL**: Each ATA session writes one line of JSON per event to `~/.ata/sessions/.../rollout-*.jsonl`. Some tests inspect this file to check what tools the model called, even when the on-screen output looks fine.
- **`workflow_dispatch`**: A GitHub Actions trigger that only fires when a human clicks a button. Used by Tier B2 to keep the model bill from running on every push.


## When something breaks

If CI fails on a TR you didn't touch:

1. Read the failure message in the GitHub Actions log. The runner prints the last 800 bytes of the captured pane on failure, which usually shows what was on screen.
2. Reproduce locally with the same tier's script. If it passes locally and fails in CI, the difference is usually file ordering (Linux vs Mac), missing CI state (no prior sessions), or a timing race.
3. If the test is flaky (passes sometimes), prefer making the predicate more permissive or adding a poll over adding more `sleep`s.
4. If the test is wrong (PLAN.md and ATA agree but the runner disagrees), fix the runner.
5. If the test is right but ATA changed, update both PLAN.md and the runner in the same PR.

If you're stuck and want to drive ATA interactively to see what's happening, the `ata-tmux-test` skill (in `~/.claude/skills/`) describes how to reproduce TUI behavior from a sandboxed agent. It's the same recipe the runner scripts use, just one step at a time.


## How this guide was built

This testing setup grew across several sessions. The PLAN.md was written first, by hand, while exploring the TUI for behaviors worth guarding. The runners were added next, translating each TR into a shell function. The three-tier split was added when we realized some tests cost real money and shouldn't fire on every push. The CI workflows came last.

Coverage now: 154 (TR, scenario) pairs across 51 of the 65 TRs in PLAN.md. The remaining 14 TRs are real blockers (hardware, build flags, non-determinism), documented in the "What we don't test" section above.

If you add new tests, the cheapest path is: write the TR in PLAN.md first, then mirror it as a shell function in the right tier's runner.
