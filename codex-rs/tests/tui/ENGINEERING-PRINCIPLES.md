# Engineering principles for fixes

How we fix things in this repo. These are derived from the strongest fixes
in our history (the `wriva` PDF/URL work is the reference example) and from
hard lessons in agentic testing. **Every fix — whether a human or an agent
writes it — must follow these.** A fix that makes the symptom disappear but
violates these is not done; it is debt.

When you (an agent or a person) are about to write a fix, read this first and
then, before you finish, check your diff against the checklist at the bottom.

---

## 1. Fix at the choke point, not the path you happened to find

Find the narrowest waist that all the relevant data or control flows through,
and fix it there once. Do not patch the one code path your repro exercised and
leave the other entry points broken.

- **Reference:** PDF file-input preparation was hooked into `run_turn` — the
  single function every turn's input passes through (direct, queued, pending) —
  not into one peripheral submit path. One hook, total coverage.
- **Anti-pattern:** wiring the same fix into `steer_input` only, so queued or
  replayed input silently misses it.
- **Test:** "If input arrived through a different door, would my fix still
  catch it?" If no, you fixed a path, not the problem.

## 2. Prefer native/platform capabilities; make workarounds unattractive

Strengthen the first-class path so neither the agent nor the code reaches for
a shell-out or manual workaround. If a fallback exists, discourage it
explicitly and make the native path the obviously-better choice.

- **Reference:** the `attach_url_files` tool reads PDFs natively as provider
  file inputs, and its description actively says *"Do not shell out to curl,
  wget, pdftotext, OCR… unless native attachment has failed."* The bug wasn't
  only "PDFs don't attach" — it was "the model curls because the native path is
  weak." Fix the native path; the workaround stops being attractive.
- **Test:** "Did I just add a fallback, or did I make the real path good enough
  that the fallback isn't needed?"

## 3. Design the failure out — don't just handle it

Prefer architecture that cannot reach the bad state over recovery code for the
bad state. Handling is the last resort, not the first.

- **Reference:** for native URL-ingestion providers, send the URL and let the
  provider fetch it, rather than downloading bytes into a local cache that can
  go stale or poisoned. The failure mode (poisoned cache blocks the send)
  cannot occur because the cache is not on the path.
- **Both are valid, in order:** first try to make the bad state unreachable;
  only if it genuinely can occur, handle it cleanly (see #5).

## 4. Centralize boundary transforms — one seam, applied uniformly

Provider-specific or format-specific shape logic belongs at the single
serialization/boundary seam, applied to every path that crosses it. Never
scatter the same transform across call sites.

- **Reference:** the PDF-block normalization runs once at the wire boundary and
  is applied uniformly across sampling, compaction, and the websocket endpoint.
  That is why compaction "just worked" — it goes through the same normalizer
  rather than needing its own copy.
- **Test:** "Is this transform duplicated anywhere? Could a new path forget to
  call it?" If yes, move it to the seam they all pass through.

## 5. Fail fast, fail typed — don't limp forward

On bad or unprocessable input, abort the operation cleanly with a specific,
typed error. Do not proceed with half-valid state, and do not retry things
that are not transient.

- **Reference:** bad file input aborts the turn early with a typed
  `ErrorEvent { BadRequest }`. Auth failures are marked non-retryable so a bad
  key fails fast instead of looping five times.
- **Corollary:** error messages name the specific thing (the file, the size,
  the limit, the host) — never a generic blob, and never leaking secrets.

## 6. Absorb domain footguns into the tool/API

Encode known gotchas — URL conversions, sensible defaults, format quirks —
into the tool or API itself so callers don't have to know them.

- **Reference:** arXiv `/abs/<id>` → `/pdf/<id>` conversion and filename
  derivation are baked into the attach tool, so the agent never has to learn
  them.
- **Test:** "Will the next caller hit the same gotcha I just worked around? Put
  the knowledge in the tool, not in a comment."

## 7. Test the observable contract, not the internals

Assert what crosses the boundary — the wire payload, the on-disk result, the
session JSONL, the tool call — not private internal state. Contract tests
survive refactors; internal-state tests rot.

- **Reference:** PDF tests assert the actual serialized wire
  (`"file_data":"data:application/pdf;base64,…"`) and the no-prefetch behavior,
  not which struct field was set.

## 8. Root-cause before patching

Investigate *why* before making the symptom go away. A plausible patch on a
misdiagnosed cause is worse than no patch — it hides the real issue.

- **Reference:** five "broken upload routing" tests turned out to be a
  test-isolation defect (a real `OPENAI_API_KEY` in the env shadowed the test
  key), not a production regression. Patching production "to fix the tests"
  would have broken correct code. The fix was to isolate the env in the tests.

## 9. Don't weaken the test or the spec to get green

Fix the production code to meet the contract. Only change a test or spec if it
encodes a genuinely wrong expectation — and then say so explicitly, with
evidence, not silently.

---

## Checklist before you call a fix done

- [ ] Did I fix the **choke point**, so every entry path is covered? (#1)
- [ ] Did I strengthen the **native path** rather than add a workaround? (#2)
- [ ] Did I **design the failure out** where possible, not just handle it? (#3)
- [ ] Is the boundary transform in **one seam**, not scattered? (#4)
- [ ] Does bad input **fail fast and typed**, with a specific message and no
      secret leak? (#5)
- [ ] Are domain **footguns absorbed into the tool/API**? (#6)
- [ ] Do my tests assert the **observable contract**? (#7)
- [ ] Did I **root-cause**, not pattern-match a patch? (#8)
- [ ] Did I avoid **weakening tests/specs** to pass? (#9)
- [ ] Is the diff **scoped** — no drive-by reformat or unrelated change?

If any box is unchecked, the fix is not done.
