# PDF ingestion — behavioral spec

This spec covers how PDFs enter a session, how their content reaches
the model, and how that pipeline degrades when the PDF is large,
scanned, corrupt, or the active provider cannot take it natively. The
history that motivates it: compaction broke in the presence of PDFs
three separate times across three months, PDF errors inside subagents
were swallowed, and each provider accumulated its own handling fixes.

Scope boundary: the compaction-with-PDFs matrix (compact a session
holding PDF content, then resume, then continue) belongs to
`session-continuity.md` and is specified there under "PDFs in
context". Do not re-spec it here; if you run this spec and continuity
has not been swept recently, run that one cell of its matrix and cite
it. This spec owns everything upstream of compaction: getting the PDF
in, getting faithful content out, and the per-provider and subagent
paths.

Like the other specs, this stays at capability level: discover exact
commands, tool names, attachment syntax, and error wording at run time
via `--help`, the slash menu, the `@` popup, and the session JSONL.
The JSONL is the ground truth — a pane that says "I read the PDF"
proves nothing about what entered the prompt.

## Capabilities and required behavior

### Attaching a PDF

There are several doors, and all must lead to the same place — the
PDF's content available to the model on the next turn:

- **Path mention in plain text.** Naming a PDF path in a prompt
  ("summarize report.pdf", relative or absolute, with trailing
  punctuation, inside parentheses or markdown emphasis) attaches the
  file. The recorded turn must show the attachment, not just the text.
- **The `@`-mention file popup.** Selecting a PDF through the popup
  attaches it the same way.
- **Pasting a PDF URL.** A pasted or typed URL to a PDF is fetched
  and its content made available. A URL that turns out not to be a
  PDF must produce a clear statement of what was actually found —
  this exact case shipped broken ("handle non pdf links better").
- All three doors must agree: same content fidelity, same size
  limits, same error wording class. A path that resolves outside the
  sandbox or to a non-existent file is a clear error, not a silent
  no-attach.

### Native attachment only — no shelling out

- PDFs reach the model through native provider file attachment, never
  by converting the PDF to text first. The agent must not shell out to
  `pdftotext`, `pdfplumber`, `PyPDF`/`pypdf`, `mutool`, `pdfimages`,
  OCR tools, or `curl`/`wget`-then-convert pipelines to read a PDF. The
  attach doors (path mention, `@`-mention, URL) and the URL-attach tool
  are the only sanctioned paths; pdfium-based rasterization for
  scanned/image-only PDFs is internal to the tool, not a shell-out the
  agent performs.
- This is judged from the session JSONL: when a PDF is in play, the
  recorded turn shows a native attachment part (or the URL-attach tool
  call), and there is no shell/exec call invoking a text-extraction or
  OCR utility on the PDF. A run that produces PDF content via such a
  shell-out instead of native attachment is a defect even if the answer
  is correct.
- The single sanctioned exception is the genuine fallback the spec
  already names: when the provider/model has no native PDF support, or
  the PDF is scanned/corrupt and the internal path fails. Even then the
  degradation must be explicit (announced, not silent), and a
  shell-out is a last resort, not the default door.

### Extraction fidelity

- **Text-layer PDFs**: a born-digital PDF with distinctive,
  greppable content (plant unique tokens) must be recallable by the
  model verbatim enough to prove the text layer was read, not
  hallucinated from the filename.
- **Scanned / image-only PDFs**: a PDF with no text layer must
  still yield content where the pipeline supports it (the binary
  ships a pdfium-based rasterization path, fetched on demand), or an
  honest statement that the document is image-only and what the
  fallback did. "I read it" followed by invented content is the
  failure this probe exists for.
- First use on a fresh HOME: the rasterization dependency may need
  to be downloaded. First-PDF-of-a-fresh-install must either work or
  report the missing dependency and how it is being resolved — never
  hang, never silently degrade to filename-only.
- Multi-page fidelity: plant tokens on the first, a middle, and the
  last page. Recall must reach all three; truncation that drops the
  tail silently is a failure even if the head reads fine.

### Size limits and honest errors

- Per-file and per-turn payload budgets exist and are
  provider-dependent. Exceeding either must produce an error that
  names the file, its size, the limit, and the provider — discovered
  wording may vary, but all four facts must be present, and the turn
  must still be usable (the oversized file dropped, the rest of the
  prompt intact, the user told).
- A limit error is a refusal, not a degradation: the model must not
  receive a truncated or mangled slice of the file presented as the
  whole document.
- The budget is enforced *before* the file is sent to the provider —
  before any upload or serialization. An over-budget or oversized file
  must never be uploaded and then 404 after the fact, and a rejected
  file reference must not be retained into a later turn (no stale
  provider file id that re-errors on the next message). When several
  files individually fit but jointly blow the per-turn budget, the
  files that overflow it are the ones dropped; the ones that fit go
  through.
- Probe the boundary, not just the far side: a file near the limit
  must go through whole.

### Per-provider delivery

Every provider had its own PDF fixes; sweep at least two of
OpenAI / Anthropic / Gemini, plus any chat-completions-style provider
the build exposes (the rasterize-to-images fallback exists for
providers whose API has no native PDF part):

- A provider with native PDF support must receive the document as a
  document — verify in the JSONL that a PDF-shaped part entered the
  request, and that recall works.
- A provider or model that cannot take native PDFs must degrade
  honestly: either a working fallback (page images, extracted text)
  with recall still functional, or an upfront refusal naming the
  provider. The forbidden outcome is the silent one — attachment
  accepted, turn succeeds, content never delivered. The code has a
  path that skips file uploads with only a debug log when the
  provider lacks PDF support; probe specifically whether the *user*
  is told.
- Switch providers mid-session after a PDF is already in context and
  send another turn: the rebuilt prompt must be valid for the new
  provider (this is the same prompt-rebuilding muscle that broke
  under compaction).

### URL-fetched PDFs and the cache

- Fetching the same PDF URL twice in one session and again in a new
  session must yield the same content; cache eviction must never
  serve a stale or truncated copy ("pdf urls: handle cache
  eviction"). Compare planted-token recall across fetches.
- A URL that 404s, redirects to HTML, or times out mid-download is a
  clear error naming the URL and the failure, and the session stays
  usable. This holds for native URL ingestion too: when the provider is
  the one fetching a `url_file` and the fetch fails (a 404 upstream), the
  failure is normalized into a clean, URL-named error and the bad URL
  attachment is dropped — the user never sees the raw provider JSON
  ("invalid_request_error … Upstream status code: 404").

### PDFs inside subagents

This broke historically ("pdf: handle errors in subagents"): errors
were swallowed. Required behavior:

- A subagent task that reads a PDF successfully must return content
  to the parent that proves the read (planted-token recall through
  the subagent's summary).
- A subagent task whose PDF read fails (corrupt file, oversized,
  unsupported provider) must surface the error to the parent —
  visible in the parent's transcript or result, never a hang and
  never a subagent that reports success around a failed read.
- The subagent honors the same size limits and provider rules as the
  parent; it must not have a side door.

### Paper pipeline integration

The research tools end-to-end path: `paper_get` (or whatever the
build's paper-fetch tool is named — discover it) produces a PDF, and
that PDF must flow into both consumers:

- the model's context (recall works on the fetched paper), and
- the document reader / reading view (the paper opens and renders;
  the reading view's own contracts live in `reading-view.md` — here
  only verify the handoff happens and the document that opens is the
  document that was fetched, not a stale or different file).

A paper that arrives as something other than a clean PDF (HTML
landing page, paywalled stub) must be reported as such, not fed to
the PDF path as garbage ("pdf urls: handle non pdf links better" is
the same bug class at the pipeline level).

## How to test it

Build a small fixture set before driving the TUI: a born-digital PDF
with planted unique tokens on first/middle/last pages, an image-only
scan of the same content, a file just under and just over the active
provider's size limit, a truncated/corrupt file with a `.pdf`
extension, and a stable public PDF URL. Generate locally where
possible so token placement is known.

Drive the real binary through tmux (recipe in the README). For every
probe the loop is: attach via one door, send a recall question, then
read the session JSONL for what was actually in the request — the
attachment part, its type, and whether the planted tokens are
reachable. Repeat the core attach+recall probe across the three doors
and at least two providers.

Then go adversarial — minimum classes, invent more:

- **Silent-drop hunting**: every attach that the UI accepts must be
  visible in the JSONL request. Any accepted-but-absent attachment
  is a finding, especially on a provider without native PDF support.
- **No shell-out for PDFs**: give the agent a PDF (path, `@`-mention,
  and URL) on a native-PDF provider and ask about its content. Grep the
  session JSONL for shell/exec calls running `pdftotext`, `pdfplumber`,
  `pypdf`, `mutool`, `pdfimages`, OCR tools, or `curl`/`wget` piped into
  a converter. The turn must carry a native attachment part (or the
  URL-attach tool call) and contain no such shell-out. A correct answer
  obtained by converting the PDF to text is still a defect. Push it:
  even when told "summarize this PDF," it must attach natively rather
  than extract text by shelling out.
- **Garbage in**: corrupt PDF, zero-byte file, HTML renamed to
  `.pdf`, a URL whose Content-Type lies. Clear error, session
  usable, nothing mangled into the prompt. A bad attachment is a
  per-attachment failure: it surfaces a clear error for that file and
  drops it, but it must not abort the rest of the turn. If the same
  prompt also requested unrelated work (for example spawning
  subagents), that work still runs. Content-validity failures (corrupt
  or wrong-magic files) are non-fatal to the turn; only a malformed
  request should fail the whole turn.
- **Boundary stacking**: several files that individually fit but
  jointly exceed the per-turn budget; the error must name the budget
  and the turn must survive.
- **Subagent error propagation**: hand a subagent the corrupt
  fixture and the oversized fixture; both failures must reach the
  parent.
- **Cross-feature handoff**: paper fetch → reading view → ask the
  model about a figure; the same document identity must hold across
  all three.
- **Compaction cell**: run the PDFs-in-context cell of the
  session-continuity matrix once (compact → continue → resume →
  continue) and cross-reference that spec in the report rather than
  duplicating its contract here.

Report per the README: issues with exact reproductions, divergences
citing the sections above, a door x provider table showing which
combinations were swept, and coverage notes. Clean up fixtures and
any downloaded rasterization artifacts you can safely remove.
