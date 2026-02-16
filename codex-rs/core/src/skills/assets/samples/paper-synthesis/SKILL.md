---
name: paper-synthesis
description: Synthesize academic papers into structured summaries and pedagogical deep dives. Use when a user asks to explain, summarize, synthesize, or deep-dive into a research paper, or when given an arXiv URL, DOI, paper title, or Zotero reference to analyze.
metadata:
  short-description: Summarize and explain research papers
---

# Paper Synthesis

Produce two complementary outputs for any research paper: a **Structured Summary** for quick reference and a **Pedagogical Deep Dive** for understanding. Default to both unless the user requests only one.

## Default Narrative Style (Walkthrough-First)

Unless the user asks for a different style, write explanations as a **self-contained technical walkthrough** for an expert reader who has not seen the paper:

- Start with the core problem in plain language before diving into architecture details.
- Explain the method as a concrete pipeline with explicit stage headers (`Stage 1`, `Stage 2`, etc.) whenever the paper uses staged training.
- For each major design choice, explicitly explain **why this choice was made** and what breaks with the simpler alternative.
- Use analogies before formal details when introducing non-obvious concepts.
- After each key equation, include both variable definitions and an intuitive explanation of what the equation is doing.
- Split long paragraphs (6+ sentences) into multiple paragraphs for readability — but never cut content to make paragraphs shorter. Use explicit transitions over dense prose.
- For multi-paper requests, include a dedicated final section in prose: `How the Papers Relate`.
- **Voice**: Use plain, direct language — simple words, short sentences, no academic stiffness. Use **second-person ("you") for procedural walkthroughs** of how the method works ("You take two frames from a video…", "You train an encoder-decoder system…"). Use **neutral third-person for framing, results, and analysis** ("LAPA tackles a fundamental bottleneck…", "The same codebook entries produce similar motions…"). Do NOT open problem statements with "You want to…" — that sounds like a tutorial, not a technical discussion.
- **Define every technical term inline on first use** in plain language before using it further. If you mention "VQ-VAE", immediately explain what it is and why it matters — never assume the reader already knows.
- **Never reference the KB in explanations.** The KB is infrastructure — the user cares about the paper, not where you stored it. Do not say "as summarized in your KB", "according to your KB card", or "the KB card for X says." Present explanations as if you understand the paper directly. The only time to mention KB cards is when the user explicitly asks about KB status, card IDs, or storage.
- **No figure-reference sections.** The reading view is text-only — images and figures cannot be displayed. Never include sections like "Figure Pointers", "How to view figures", or "Key Figures" that tell the user to look at specific figures by number. Instead, describe what each important figure shows inline in the narrative (e.g., "The architecture diagram in the paper shows three stages connected by…"). This applies to `present_reading_view` content and chat explanations alike.
- **Use concrete worked examples with specific numbers** to build intuition (e.g., "8 possible values × 4 positions = 4,096 latent actions"). Specificity builds intuition faster than abstraction. However, keep model variant names, exact tensor shapes, hyperparameter values, and architecture identifiers out of the narrative paragraphs — collect them in the Details block (see next rule).
- **Details block at the end of each subsection.** After the narrative paragraphs of each subsection (each Stage, each major section), add a **Details:** line that collects reference specifics: model names and variants, exact dimensions and tensor shapes, hyperparameter values, tokenizer identifiers, layer counts, hidden dims, optimizer settings, etc. The narrative should be fully understandable without reading the Details block — it is a reference appendix for precision, not part of the conceptual flow. Example:
  > **Details:** Base model: Cosmos-Predict2-2B Video2World. Tokenizer: Wan2.1. Input: (1+T)×H×W×3 → (1+T')×H'×W'×16 (T'=T/4, H'=H/8, W'=W/8). Text encoder: T5-XXL.
- **Explain concepts completely in place.** When a concept is non-obvious, explain it right where it appears rather than deferring to a later section. If Stage 1 uses a VQ-VAE, explain VQ-VAE right there in Stage 1.

### Basics-First Contract (Mandatory by default)

For explain-style requests, always lead with conceptual clarity before formal detail:

- Use this opening flow per paper:
  1. `The problem` (plain language, 3-6 sentences)
  2. `The Core Idea` (what the paper changes, at a high level)
  3. `Stage-by-stage walkthrough` (`Stage 1`, `Stage 2`, `Stage 3` where applicable)
  4. `Why This Matters` (why this is better than a simpler baseline)
  5. `Key Results` (numbers + one-sentence interpretation each)
  6. `Limitations` (practical boundary conditions)
- If equations are used, first explain intuition in words, then show equation, then define symbols.
- Avoid opening with dense implementation detail, notation, or long architecture dumps.
- Prefer concrete examples (e.g., "frame before / frame after", "replace latent head with action head") over abstract phrasing.
- **Progressive disclosure:** each concept must be fully grounded before the next builds on it. Never forward-reference a mechanism that hasn't been explained yet.
- **No undefined jargon or acronyms.** Every abbreviation and technical term gets a plain-language gloss on first use — even common ones like "MLP" ("a small feedforward neural network") or "DiT" ("Diffusion Transformer").

### Mandatory Explanation Completion Contract

**This contract applies to the main agent only.** Subagents write KB cards and return — they never invoke cross-paper-report or `present_reading_view`.

When the user asks to **explain** a paper (walkthrough, deep dive, understanding, synthesis), shallow chat-level summaries are forbidden. You MUST complete one of these paths:

1. **KB-first reuse** — Call `kb_status` and `kb_search` (in parallel) for matching paper cards (title, DOI, arXiv ID). If a card with a Deep Dive exists, read it via `kb_read_card` and present via `present_reading_view`. Done.

2. **Synthesize path** — If no matching card exists, resolve the paper identifier (see Pre-Synthesis routing), launch a subagent to write the card, then read the new card via `kb_read_card` and present via `present_reading_view`. For multiple papers, launch subagents in parallel.

Do NOT invoke cross-paper-report as part of paper-synthesis. Cross-paper-report is a separate skill the user triggers for explicit comparison requests. Do NOT stop after a brief inline explanation — completion requires `present_reading_view`.

## Execution: Use Subagents

**Always launch a subagent for each paper.** This is mandatory, not optional:

- **Single paper**: Launch one subagent. This keeps the full paper text out of the main conversation context, preventing context pollution.
- **Multiple papers**: Launch one subagent per paper, **in parallel**. This dramatically speeds up multi-paper requests.

### Subagent Prompt Construction

Each subagent prompt MUST include `$paper-synthesis` — this triggers automatic injection of the full 300+ line skill instructions into the subagent's context. Without it, the subagent will improvise — using shell commands to extract text, writing shallow summaries, and skipping figure extraction.

**Do NOT write a custom prompt that describes what the subagent should do.** If you write "You are summarizing a research paper…" or similar, the subagent will NOT have the skill instructions and will fall back to shell-based approaches. Use the template below **verbatim** (only fill in the bracketed fields):

> The $paper-synthesis skill instructions are loaded in your context. Execute every section: Pre-Synthesis, Type 1 Structured Summary, Type 2 Pedagogical Deep Dive (with the Default Narrative Style and Basics-First Contract), Figure Extraction, Depth Enforcement, and KB Card Storage. Skip the "Execution: Use Subagents" section — you ARE the subagent. Do NOT invoke `cross-paper-report` — the main agent handles that after you return.
>
> CRITICAL: To read the paper, use `attach_url_files` with the PDF URL. Do NOT use shell commands (curl, wget, pdftotext, python) to download or extract PDF text. The model reads PDFs natively via attach_url_files.
>
> Paper: [identifier — URL, DOI, arXiv ID, or Zotero item key]
> KB path: [value from kb_status]
> [For Zotero papers: item key, downloaded PDF path, and any notes already retrieved]

Do NOT manually reconstruct the workflow or style rules in the subagent prompt. Do NOT write your own custom prompt — use the template above exactly. The `$paper-synthesis` mention causes the skill to be auto-loaded into the subagent's context. If you summarize the instructions yourself instead, the subagent will miss critical rules (like using `attach_url_files` instead of shell commands).

### What Subagents Return

Each subagent writes the KB card directly via `kb_write_card` (and extracts figures if available). After completing, the subagent returns a **concise report** to the main agent containing:
- Card ID that was written
- Paper title, authors, year
- 3-5 sentence summary of the core contribution
- Key architectural choices (backbone type, scale, notable modules)
- Key training details (stages, data sources, losses)
- Whether figures were extracted and how many

The subagent does NOT need to return the full Type 1 + Type 2 text — that content lives in the KB card. The subagent's report is just enough for the main agent to inform the user and produce a cross-paper comparison.

### Main Agent Role

The main agent orchestrates subagents and handles presentation. Subagents write KB cards — they never produce final user-facing output or invoke cross-paper-report.

#### Single-Paper Path

When the user asks about ONE paper (explain, walkthrough, deep dive, summarize):

1. Call `kb_status` and `kb_search` **in parallel** to check for an existing card. Optionally read `research-context.md` in the same parallel batch (via `kb_read_file` at path `research-context.md`) to tailor the subagent prompt.
2. If a card with a Deep Dive exists → read it via `kb_read_card` → present via `present_reading_view` → **done, skip remaining steps.**
3. Resolve the paper identifier using the Pre-Synthesis routing rules (see below). This produces a PDF URL, DOI, or arXiv ID. **Do NOT search Zotero unless the user explicitly mentions Zotero or their library.**
4. Launch ONE subagent with the standard template (see Subagent Prompt Construction). Include the resolved identifier, `kb_path`, and any research context priorities (see Personalization below).
5. When the subagent returns, read the newly written card via `kb_read_card`.
6. Present the Deep Dive content via `present_reading_view`. Title: the paper name (never card IDs or "KB" references).

This path should complete in 3–4 tool-call round-trips in the main agent. Do NOT add unnecessary calls (no `zotero_get_collections`, no `paper_search` alongside `zotero_search`, no reading the SKILL.md via shell commands).

#### Multi-Paper Path

When the user asks about MULTIPLE papers or a broad topic:

1. Call `kb_status` and `kb_search` for each paper **in parallel**. Optionally read `research-context.md` in the same parallel batch to tailor subagent prompts toward user priorities.
2. For papers that already have cards, skip synthesis. Only synthesize papers that lack cards.
3. Resolve identifiers for missing papers using Pre-Synthesis routing.
4. Launch one subagent per missing paper, **in parallel**.
5. Collect subagent reports and tell the user which cards were written and a brief highlight per paper.
6. If the user wants comparison, suggest `$cross-paper-report` as a follow-up. Do NOT run it automatically.

#### Personalization via Research Context

If `research-context.md` exists at the KB root (read via `kb_read_file` at path `research-context.md`), use it to tailor the walkthrough:

- **Priorities** affect emphasis: If the user cares about inference latency, spend more words on the inference pipeline and latency numbers. If they care about data efficiency, expand the training data discussion. The walkthrough covers everything, but the priority sections get deeper treatment (more paragraphs, more worked examples).
- **Not Interested In** affects framing: If the user has dismissed pure RL approaches, don't spend a paragraph motivating RL vs. imitation learning — state the approach and move on. Still cover the method fully, but don't sell the user on something they've already decided against.
- **Framings That Work** affect style: If the user responds to tradeoff framing, structure explanations as "you get X but you lose Y." If they prefer mechanical analogies, use those. If they prefer concrete numbers, lead with numbers before abstractions.
- **Project context** affects connections: If the user is building a bimanual manipulation pipeline, point out which parts of the paper are directly relevant to their setup and which are tangential.

When passing research context to a subagent, include a brief summary in the subagent prompt:
> User priorities: [list from research-context.md Priorities section]
> Emphasize: [sections relevant to priorities]
> User project: [brief from Project section, if relevant]

If `research-context.md` doesn't exist, produce the standard walkthrough — no personalization needed.

#### After Synthesis: Interactive Workflow

After synthesis is complete and the user starts chatting about the papers, the KB cards should grow with the conversation. The following skills compose with paper-synthesis:

- **`$kb-update`** — When follow-up Q&A produces insights not in the card, invoke kb-update to persist them back to the card's Discussion Notes section. This happens naturally during conversation — no explicit invocation needed from the user.
- **`$research-briefing`** — When the user wants a quick orientation of multiple papers before diving deep, suggest this as an alternative to cross-paper-report. Produces a concise 2-4 page overview.
- **`$conversation-report`** — When the user has been chatting about papers and wants to capture the discussion as a document, suggest this. It organizes the conversation's Q&A into a focused report.

#### Post-Synthesis Housekeeping

After completing a synthesis (card written + reading view presented), do these in the background — they should not block the user from interacting with the reading view:

**1. Journal entry** — Append a brief entry to `research-journal.md` at the KB root via `kb_write_file`. If the file doesn't exist, create it. Prepend (newest first):

```markdown
## [Date] — Synthesized: [Paper Title]

### Action
- Synthesized [paper title] into KB card `[card-id]`
- Source: [URL or "Zotero item XDBQLKYV" or "paper_search"]

### Cards Touched
- [card-id] (created)
```

For multi-paper synthesis, list all papers in one entry. Keep it short — 5-10 lines max.

**2. Research context detection** — During the synthesis interaction (including follow-up questions), watch for preference signals:
- User asks "skip the RL motivation" or "I know how transformers work" → They're expert in that area, note in research-context.md under Framings That Work
- User asks "focus on the inference pipeline" → Priority signal
- User says "how would this work for my bimanual setup?" → Project context signal
- User responds positively to an analogy or framing → Framings That Work signal

When you detect a preference signal, offer briefly: "Want me to note that [preference] in your research context so future walkthroughs adapt?" If yes, read `research-context.md` (create if needed), merge the new item, write it back. If no or ignored, move on — never block on this.

#### Post-Reading-View: Persist Follow-Up Insights

When the user asks follow-up questions inside the reading view (via `append_to_section`), those answers are added to the ephemeral reading view document — they are not automatically saved to the KB card. The reading view is a display surface, not storage.

When the user exits the reading view and returns to the main conversation:

1. **Check if the reading view Q&A produced insights not already in the KB card.** Typical examples: the user asked "how does X handle Y?" and got a targeted explanation, or asked about a specific failure mode, or asked for a comparison with another method.
2. **If yes, offer to update the KB card:** "The questions you asked about [topics] produced explanations not in the original card. Want me to add them to the card's Discussion Notes so they're available next time?"
3. **If the user agrees, use the `$kb-update` protocol** — read the card, append the follow-up insights under Discussion Notes with today's date, write the card back.
4. **Append follow-up insights to the journal** — If insights were persisted to cards, also append a brief journal entry noting the follow-up: `### Follow-up: [Paper Title]` with bullets for what was discussed.

This is lightweight and non-blocking — if the user moves on to a different topic, don't interrupt. But if there's a natural pause or the user explicitly returns from the reading view, this is the moment to offer.

### Cross-Paper Comparison (Optional, Multi-Paper Only)

When the user explicitly asks to **compare** multiple papers or wants a **cross-paper report**, suggest running the `cross-paper-report` skill after synthesis is complete. This is a separate skill — never a mandatory follow-up to paper-synthesis.

Cross-paper-report produces:
1. **Deep comparative narrative** tracing shared ideas, divergences, and field trajectory
2. **LaTeX PDF** with TikZ diagrams compiled via `latex_compile`

To trigger this, launch a subagent with the following prompt:

> Invoke the `cross-paper-report` skill for cards `paper-lapa` and `paper-groot-n1`. The KB path is `/path/to/knowledge-base`. Follow the skill's full workflow — both deliverables (narrative + PDF) are mandatory.

Do NOT run cross-paper-report for single-paper explanation requests. The KB card's Deep Dive (1000-2500 words) is the explanation — presenting it via `present_reading_view` is sufficient.

## Pre-Synthesis: Obtain the Full Paper

Before synthesizing, always attempt to read the full paper text. Choose **exactly one** path based on what the user provided.

**Route selection (choose ONE — do NOT call tools from multiple paths):**

- **User gave a URL** (arXiv link, DOI link, PDF URL) → **Path A**. Use the URL directly.
- **User gave a paper title or author names** (no URL, no Zotero mention) → Use `paper_search` to find the arXiv ID or DOI, then **Path A**.
- **User mentions Zotero, a collection, or "my library"** → **Path B**.

Do NOT call `zotero_search` or `zotero_get_collections` unless the user explicitly references Zotero. Do NOT call both `paper_search` and `zotero_search` for the same paper.

**CRITICAL: Never use shell commands to download or extract PDF text.** Do NOT use `curl`, `wget`, `pdftotext`, `pdfimages`, `python` scripts, or any other Bash-based approach to fetch or convert PDFs. The model can read PDFs natively when they are attached via `attach_url_files`. Shell-based text extraction loses figures, tables, formatting, and mathematical notation — it is strictly inferior and is forbidden.

### Path A: arXiv URL or DOI (default)

1. If given an arXiv `/abs/` URL, convert it to `/pdf/` (e.g. `https://arxiv.org/abs/2503.14734` becomes `https://arxiv.org/pdf/2503.14734`).
2. Use `attach_url_files` to fetch the PDF. After it succeeds, the PDF content is injected into your conversation context automatically — you can read and analyze it immediately. Do not search for a downloaded file on disk or use shell commands to extract text. If available, use `paper_get` to retrieve metadata (title, authors, abstract) as supplementary context.
3. If PDF fetch fails, fall back to the abstract from `paper_get` and note in output: "Based on abstract only; full text unavailable."
4. If neither source is available, clearly state this limitation upfront.

### Path B: Zotero (when user mentions Zotero, a collection, or their library)

1. Use `zotero_search` to find the paper(s) by title, author, or topic. Only call `zotero_get_collections` if the user names a specific collection — do NOT scan all collections speculatively. If the user names a collection, use `zotero_get_collection_items` directly.
2. For each paper found, call `zotero_get_item` with `include_attachments=true` and `include_fulltext_resolution=true`.
3. If `document_resolution.preferred_url` (PDF URL) is present, fetch the paper with `attach_url_files` and treat that attached document as the primary source (this preserves figures/tables). After `attach_url_files` succeeds, the PDF content is injected into your conversation context automatically — you can read and analyze it immediately. Do not search for a downloaded file on disk or use shell commands to extract text.
4. If no URL is available but `document_resolution.local_path` is present, use that local PDF path as the primary source.
5. Do not call `zotero_get_fulltext` for paper synthesis. Indexed fulltext is lossy (no figures/tables) and is not an acceptable primary source when PDF resolution is required. Similarly, do not use `pdftotext` or any shell-based text extraction — the model reads PDFs natively via `attach_url_files`.
6. Optionally call `zotero_get_notes` to retrieve the user's annotations and highlights — weave these into the synthesis where relevant (e.g. "The authors note X, which the reader flagged as particularly relevant because...").
7. If neither `preferred_url` nor `local_path` is available, stop and report this as a Zotero metadata inconsistency instead of switching to indexed fulltext.

When the user asks to analyze multiple papers from Zotero (e.g. "synthesize my Zotero collection on diffusion"), launch one subagent per paper in parallel. After all subagents complete, present summaries and suggest `$cross-paper-report` if the user wants a comparative deep dive.

## Type 1: Structured Summary

Use bullet points under these headers:

### Metadata
- **Title**, **Authors**, **Year**, **Venue** (if known), **arXiv ID / DOI**

### Problem & Motivation
- What gap or limitation does this paper address?
- Why does it matter?

### Architecture
- **Backbone**: Name, type (ViT, DiT, U-Net, etc.), layer count, hidden dim, attention heads, patch size
- **Input encoding**: How each modality (vision, language, proprioception) is tokenized/embedded — token dims, sequence lengths, positional encoding
- **Output heads**: What the model predicts (actions, tokens, flow vectors), output dimensions, chunk sizes, prediction horizons
- **Key modules**: Any non-standard components (cross-attention, codebook quantization, MoE routing, adaptive normalization)
- **Parameter count**: Total and per-component if available

### Training Pipeline
1. Number each training stage (pretrain → finetune → co-finetune, etc.)
2. For each stage: objective, data source and scale, frozen/unfrozen components, duration
3. Loss functions with mathematical form and weighting
4. Optimizer, learning rate schedule, batch size, key hyperparameters
5. Inference pipeline: full forward pass from observation to action, denoising steps, latency, control frequency

### Core Method
1. Numbered steps describing the approach at a high level
2. Note any novel components vs. standard building blocks
3. Highlight what makes the design non-obvious — what breaks with the simpler alternative

### Key Results
- Report specific numbers (accuracy, F1, latency, etc.) with baselines for comparison
- Note datasets and evaluation protocols

### What's Novel
- What is genuinely new vs. incremental improvement?

### Limitations
- Authors' stated limitations plus any you identify from the methodology

## Type 2: Pedagogical Deep Dive

Write flowing narrative prose. Never use bullet points in the deep dive.

**Hard depth floor: the Deep Dive must be at least 1000 words.** Architecture-heavy papers should reach 1500-2500 words. If your Deep Dive is shorter than 1000 words, you have written a summary, not a deep dive — expand before saving.

### Elements to Cover

The Deep Dive should cover these elements. The agent decides the structure and ordering — organize them in whatever sequence tells the clearest story for this paper. Do NOT follow these as a rigid numbered template. Integrate equations, analogies, and diagrams at the points where they naturally arise in the narrative, not in separate sections at the end.

- **Framing.** Open by situating the paper relative to prior work: "Where [prior approach] addresses X by doing Y, this paper asks a different question: Z." Establish why the reader should care.

- **Architecture.** Describe the model architecture in narrative prose: backbone type and role, input tokenization/embedding, output heads and decoding, and non-standard modules (cross-attention, codebook quantization, MoE routing, adaptive normalization) with why the simpler alternative would fail. End with a **Details:** block (model variant, layer counts, hidden dims, parameter counts, sequence lengths, etc.).

- **Training pipeline.** Walk through every training stage: objective, data and why, frozen vs. trained components, rationale. Include loss functions with mathematical form and intuitive explanation. Explain the inference pipeline end-to-end. End with a **Details:** block (optimizer, LR schedule, batch size, step counts, GPU-hours, latency, control frequency, etc.).

- **Method walkthrough.** This is the core of the Deep Dive and should be the longest part. Walk through the method stage by stage with worked examples. Collect exact tensor shapes and hyperparameters in **Details:** blocks.

- **Equations with intuition.** Present equations **at the point in the narrative where they naturally belong** — inside a training stage, inside the architecture discussion, wherever they arise. Do not collect them into a separate section at the end. After each equation, define every variable and write a full-paragraph intuition (3–5 sentences).

- **Analogies.** Introduce analogies **before** the formal explanation, right where the concept appears. Do not defer them to a separate section.

- **Results interpretation.** Interpret what ablations reveal about which components matter. Do not merely restate numbers.

- **Limitations woven in.** Weave failure modes and limitations into the narrative at the points where they arise from specific design choices, not in a separate section.

- **Connections.** Cross-reference related work meaningfully: "This is analogous to [X] in [related paper], but differs in [specific way]."

### Depth Calibration

Your stage walkthroughs should match this quality level (from a LAPA explanation):

> You take two consecutive frames from a video — before and after. Something happened between them. A hand moved, an object shifted. You don't know what the "action" was formally, but frame 2 is the result of some action applied to the state in frame 1.
>
> You train an encoder-decoder system. The encoder sees both frames and compresses "what changed" into a short discrete code — say [3, 2, 0, 1]. The decoder takes frame 1 plus that code and tries to reconstruct frame 2. If the reconstruction is good, the code must be capturing the essential action.
>
> These codes come from a VQ-VAE (Vector Quantized Variational Autoencoder). A VQ-VAE maintains a codebook — a fixed-size dictionary of learned embedding vectors. The encoder outputs a continuous vector, but it gets snapped to the nearest codebook entry via nearest-neighbor lookup. [...] With 8 possible values per position and 4 positions, you get 8^4 = 4,096 possible latent actions, each ending up semantically meaningful: "move down-left," "rotate right," "stay still," etc.

Key qualities to replicate:
- **Specific numbers inline** (8^4 = 4,096; 7B parameters; 50.1% vs. 43.9%)
- **Analogies before formal explanations** ("Think of this as building an action dictionary from scratch")
- **WHY behind each design choice**, not just WHAT
- **What breaks with the simpler alternative** explained concretely
- **Narrative flows as connected prose**, not bullet lists

**Per-stage depth rule: each stage must be at least 2–3 full paragraphs.** Each stage should cover:
- What concretely happens (use worked examples; collect exact tensor shapes in the **Details:** block)
- Why this design choice was made — what breaks with the simpler alternative
- How the output of this stage feeds into the next

A stage description like "The latent head is removed and replaced with a new robot-action head" is a summary, not a walkthrough. A proper version explains what the old head looked like, what the new head's shape is (e.g., "7 dimensions × 256 bins for a 7-DOF arm"), why the old head is disposable, and why fine-tuning converges fast.

Similarly, "Robot vectors are normalized to [-1,1], tiled, and inserted into latent slots" is a summary. A proper version explains the concrete mechanics: take a 50×14 action array (700 numbers), normalize each to [-1,1], copy the 700 numbers ~23 times to fill a 32×32×16 = 16,384-element volume, then slot it into the sequence as if it were an image latent.

## Figure Extraction

If `pdf_extract_figures` is available and a PDF was successfully fetched:

1. Call `pdf_extract_figures` with `pdf_url` set to the PDF URL and `output_dir` set to a **temp directory** (e.g., `/tmp/pdf-figures-<card-id>/`). Do NOT extract directly into `<kb_path>/assets/` — that would leave unselected figures permanently in the KB.
2. **Filter** using `quality_hints`:
   - **Reject** figures flagged "likely text/table screenshot" — these are almost always misidentified text regions or rendered tables, not real diagrams.
   - **Deprioritize** figures with "high whitespace, may be a text region" unless the caption clearly describes a real diagram (e.g., "Figure 3: Architecture overview").
   - **Skip** figures flagged "extreme aspect ratio" — these are typically page banners, headers, or decorative rules.
   - **Prefer** figures with empty `quality_hints` (passed all checks) and meaningful captions.
3. **Select 2-5 figures** from the remaining candidates using this priority order:
   1. **Architecture and method diagrams** (highest priority) — system overviews, block diagrams, pipeline schematics, model architecture figures. These are the most valuable because they visually explain *how the method works*. Look for captions mentioning "architecture", "overview", "pipeline", "framework", "method", "approach", or stage/component names.
   2. **Training pipeline and data flow diagrams** — figures showing training stages, data processing, loss computation, or inference pipelines.
   3. **Qualitative result visualizations** — side-by-side comparisons, generated outputs, attention maps, failure mode illustrations. These show *what the model does* concretely.
   4. **Quantitative result charts** (lowest priority) — bar charts, line plots, tables comparing numbers. Only include these if fewer than 2 figures were selected from higher-priority tiers, or if the chart reveals something the narrative cannot convey in text (e.g., a striking scaling curve).
   - When in doubt between a results chart and an architecture diagram, always pick the architecture diagram. The Deep Dive narrative can describe numerical results in text, but cannot substitute for a visual system overview.
4. **Move only the selected figures** into the KB assets directory: `mkdir -p <kb_path>/assets/<card-id>/ && cp <selected-figure-paths> <kb_path>/assets/<card-id>/`. Then delete the temp directory (`rm -rf /tmp/pdf-figures-<card-id>/`).
5. Pass the selected figures to `kb_write_card` via the `figures` field, using their new paths under `assets/<card-id>/`, with a short `caption` for each and the `page` number.

If `pdf_extract_figures` is not available, skip this phase silently.

## Depth Enforcement (Before Saving)

Before writing the KB card, verify depth mechanically:

1. **Deep Dive word count.** The Deep Dive section (Type 2) must be at least 1000 words. If it is shorter, you have written a summary, not a deep dive — expand before saving. Architecture-heavy papers should reach 1500–2500 words.
2. **Stage walkthrough depth.** Every stage in a stage-by-stage walkthrough must be at least 2–3 full paragraphs. If any stage is a single sentence (e.g., "Stage 2 trains a VLM to predict latent actions"), expand it with what happens concretely, why this design was chosen, and what breaks with the simpler alternative.
3. **Equation intuitions.** Every equation in the Key Equations section must have variable definitions plus a full-paragraph intuition (3–5 sentences). One-liners like "continuous motion is snapped to a symbol" are insufficient.
4. **Concrete numbers.** The walkthrough must include specific numbers from the paper (parameter counts, dataset sizes, accuracy figures, compression ratios, tensor dimensions) — collected in Details blocks but referenced for intuition in the narrative.

If any checkpoint fails, expand the relevant section before proceeding to `kb_write_card`. A shallow card propagates shallowness into every downstream report that reads it.

## KB Card Storage

If `kb_write_card` is available, store the synthesis as a KB card.

### Card Frontmatter
```yaml
title: "Paper: <paper title>"
source_type: paper
refs:
  - "<arXiv ID or DOI>"
tags:
  - <primary domain, e.g. "nlp", "cv", "rl">
  - <specific topic, e.g. "attention", "diffusion">
contributed_by: paper-synthesis
```

### Card Body Structure
```markdown
## Summary
<Type 1 structured summary content>

## Architecture
<Detailed architecture description: backbone, input encoding, output heads, key modules, parameter counts>

## Training Pipeline
<Stage-by-stage training: objectives, data, losses, optimization, inference pipeline>

## Deep Dive
<Type 2 pedagogical narrative — minimum 1000 words. Must include multi-paragraph
stage walkthroughs with concrete numbers/dimensions, "why this / what breaks" for each
design choice, analogies before formalism, and results interpretation. This section is the
primary content of the card — it must be a self-contained walkthrough, not a summary.
Equations, analogies, and diagrams should be integrated throughout the narrative at
the points where they naturally arise — not collected into separate sections at the end.>

## Connections
<List 3-5 related papers with one-line descriptions of the relationship>
```

If `kb_write_card` is not available, produce both types directly in the chat response.

## Presentation

IMPORTANT: When the synthesis is complete, you MUST call `present_reading_view` to present it in sectioned reading mode instead of outputting text directly. Do NOT stream the report as regular text. Set `document_id` to a unique slug, `title` to the paper title or synthesis name, and `content` to the full markdown with `## ` headings for sections. End your response immediately after calling this tool.

When the user asks follow-up questions about a specific section, use the most efficient update tool:
- `append_to_section` — to add new information at the end of a section (most common for follow-up questions)
- `patch_document_section` — to change specific text within a section (for corrections or targeted edits)
- `update_document_section` — to fully rewrite a section (only when the entire section needs to change)

## Graceful Degradation

- **No KB tools configured**: Output both types in chat; skip card storage.
- **No `paper_get` available**: Rely on `attach_url_files` for the PDF; extract metadata manually from the paper text.
- **PDF download fails**: Synthesize from the abstract and any user-provided context. Clearly note the limitation.
- **User provides only a title**: Search for the paper using available tools before synthesizing. If not found, ask for a URL or arXiv ID.
- **Zotero document resolution unavailable**: Report the item key and missing resolution fields (`preferred_url`, `local_path`) as a Zotero metadata inconsistency; do not switch to indexed fulltext.
- **No Zotero tools configured**: If the user mentions Zotero but tools aren't available, tell them Zotero integration requires API key configuration and fall back to Path A.
