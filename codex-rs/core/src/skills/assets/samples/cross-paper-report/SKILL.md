---
name: cross-paper-report
description: Generate integrated cross-paper explanations and PDF reports from KB cards. Use when a user asks to explain, compare, synthesize, understand, or deep-dive into one or more knowledge base cards, or asks how cards relate or work together.
---

# Cross-Paper Report

> **When to use this skill vs. alternatives:**
> This skill produces exhaustive deep reports (800+ words per paper, TikZ diagrams, full comparative synthesis). For quick orientations, use `$research-briefing` instead — it gives a 2-4 page overview with 3-5 sentences per paper. For conversation-derived reports that capture what you specifically discussed, use `$conversation-report` instead. Use this skill when the user wants a complete technical reference document.

You MUST produce two deliverables — no exceptions, no shortcuts:

1. **Deep narrative explanation** in the chat (focal cards: 800–2500 words; supporting cards: 400–600 words; see Tiered Depth Strategy)
2. **LaTeX PDF** compiled via `latex_compile` with TikZ diagrams

A short summary is NEVER acceptable. This task is NOT complete until both deliverables exist. Do NOT ask the user whether they want a PDF — always produce it. The PDF is the archival artifact; the chat shows it live. No separate markdown file — the PDF replaces it.

## Default Narrative Style (Self-Contained Walkthrough)

Unless the user requests a different format, write in a teaching-first style that is self-contained and easy to follow:

- Assume the reader has not read the paper/card.
- Open each card explanation with the concrete problem in plain language.
- For paper-like cards, explain the method as a stage-by-stage pipeline (`Stage 1`, `Stage 2`, `Stage 3`) when applicable.
- For each technical mechanism, include:
  - what it is,
  - why it was needed,
  - what simpler alternative would fail and why.
- Introduce analogies before dense formalism for non-obvious components.
- Split long paragraphs (6+ sentences) into multiple paragraphs for readability — but never cut content to make paragraphs shorter. Use explicit transitions (`Now`, `Next`, `At inference`, `Why this matters`).
- Keep equations, but always follow each with variable definitions plus an intuitive explanation.
- For multi-card outputs, close with a dedicated prose section named `How the Papers Relate` (not just a table).
- **Voice**: Use plain, direct language — simple words, short sentences, no academic stiffness. Use **second-person ("you") for procedural walkthroughs** of how the method works ("You take two frames…", "You delete the old MLP head and initialize a new one…"). Use **neutral third-person for framing, results, and analysis** ("LAPA tackles a fundamental bottleneck…", "The same codebook entries produce similar motions…"). Do NOT open problem statements with "You want to…" — that sounds like a tutorial, not a technical discussion.
- **Define every technical term inline on first use** in plain language before using it further. If you mention "flow matching" or "VQ-VAE", immediately explain what it is and why it matters — never assume the reader already knows.
- **Use concrete worked examples with specific numbers** to build intuition (e.g., "8 possible values × 4 positions = 4,096 latent actions"). Specificity builds intuition faster than abstraction. However, keep model variant names, exact tensor shapes, hyperparameter values, and architecture identifiers out of the narrative paragraphs — collect them in the Details block (see next rule).
- **Details block at the end of each subsection.** After the narrative paragraphs of each subsection (each Stage, each major section), add a **Details:** line that collects reference specifics: model names and variants, exact dimensions and tensor shapes, hyperparameter values, tokenizer identifiers, layer counts, hidden dims, optimizer settings, etc. The narrative should be fully understandable without reading the Details block — it is a reference appendix for precision, not part of the conceptual flow. Example:
  > **Details:** Base model: Cosmos-Predict2-2B Video2World. Tokenizer: Wan2.1. Input: (1+T)×H×W×3 → (1+T')×H'×W'×16 (T'=T/4, H'=H/8, W'=W/8). Text encoder: T5-XXL.
- **Explain concepts completely in place.** When a concept is non-obvious, explain it right where it appears rather than deferring. If a training stage uses a VQ-VAE, explain VQ-VAE right there.

### Basics-First Contract (Mandatory by default)

Before diving into full technical detail, each card explanation should establish basics clearly:

- Start with:
  1. `The problem` (plain-language bottleneck)
  2. `The Core Idea` (what this method changes)
  3. `Stage-by-stage walkthrough` (`Stage 1`, `Stage 2`, `Stage 3` if applicable)
  4. `Why This Matters` (what simpler baseline fails and why)
  5. `Key Results` (numbers + interpretation)
  6. `Limitations` (what still fails or is uncertain)
- In this basics-first section, avoid symbol-heavy exposition.
- Present equations only after intuition has been established.
- Use concrete examples and analogies before formal abstractions.
- **Progressive disclosure:** each concept must be fully grounded before the next builds on it. Never forward-reference a mechanism that hasn't been explained yet.
- **No undefined jargon or acronyms.** Every abbreviation and technical term gets a plain-language gloss on first use — even common ones like "MLP" ("a small feedforward neural network") or "DiT" ("Diffusion Transformer").

## Prerequisites

Call `kb_status` first. The response includes `kb_path` — use that value wherever this document says `<kb_path>`.

## Scale Detection

After reading all requested cards, check the count:

- **Standard mode** (≤ 12 cards): Follow Phases 1–3 as written below. Every card gets a full deep walkthrough in one pass.
- **Large-set mode** (> 12 cards): Jump to the **Large-Set Synthesis** section at the end of this document. Do NOT attempt standard Phases 1–3 for large sets — producing 800+ words per card for 20+ papers in a single agent pass will collapse into shallow summaries that violate the depth contract. The large-set workflow uses clustering and parallel subagents to preserve per-card depth at scale.

## Phase 0: Related Card Discovery

After reading the requested cards but before writing explanations, check whether the KB contains other cards that are topically relevant and would strengthen the report.

1. Call `kb_list_cards` to see all available cards.
2. For each requested card, scan its `tags`, `connections`, and the topics mentioned in its body. Identify other KB cards that share tags, are cited in connections, or address closely related methods.
3. If relevant unrequested cards exist, present them to the user:
   > "Your KB also has cards for [list] which are related to this topic. Want me to include any of them? Adding them would strengthen the comparison by covering [specific dimension, e.g., 'alternative action representations' or 'data augmentation methods']."
4. If the user approves additions, include them in the card set for Phases 1–2. Re-check Scale Detection with the updated count.

This step prevents narrow reports that miss important context available in the KB. Skip only if the user explicitly says "only these cards."

**Subagent context:** Skip Phase 0 entirely when running as a cluster subagent in the Large-Set Synthesis workflow. Cluster subagents work only on their assigned cards — discovering additional cards would cause cross-contamination between clusters. Phase 0 runs only in the main agent before clustering.

## Phase 1: Deep Technical Explanation of Each Card

Read all requested cards via `kb_read_card` (or `kb_list_cards` + `kb_read_card` when the user says "all" or refers to a broad set).

### Coverage-First Principle

**Coverage is mandatory. Never drop cards to save depth budget.** If the user asks about a topic and 10 cards are relevant, all 10 must appear in the report. Do NOT select a subset of "representative" cards and skip the rest — that produces a report with blind spots. If producing full depth for every card exceeds a single pass, use the Tiered Depth Strategy below or the Large-Set Synthesis workflow.

The wrong tradeoff: "I'll only cover 3 of 10 relevant papers so I can go deep on each." The right tradeoff: "I'll cover all 10 papers, going deepest on the most important ones."

### Tiered Depth Strategy

When the card set is moderate (5–12 cards), assign each card to a depth tier:

- **Focal cards** (the 2–4 most novel, complex, or central papers): Full 800–2500 word treatment with multi-paragraph stage walkthroughs, worked examples, full-paragraph equation intuitions, and Details blocks. These are the papers where the reader needs to understand the complete mechanism.

- **Supporting cards** (the remaining papers): Substantive 400–600 word treatment covering: the problem (2–3 sentences), core idea and what makes it different (1 paragraph), key mechanism with enough detail to understand what it does and why (1–2 paragraphs), key results with interpretation (1 paragraph), and how it connects to the focal cards (2–3 sentences). This is shorter than a focal walkthrough but far more than a one-sentence summary.

**Tier assignment criteria:** Assign to focal tier based on: (1) novelty of the method, (2) complexity requiring detailed explanation, (3) centrality to the user's question, (4) architectural uniqueness vs. being a variant of another card. Present the tier assignment to the user before writing.

**Subagent context:** When running as a cluster subagent, treat all cards as focal (no tiering) and skip user confirmation. Clusters are already sized (4–8 cards) for full-depth treatment.

**For 1–4 cards:** All cards are focal. No tiering needed — every card gets full 800+ word treatment.

**For 13+ cards:** Use the Large-Set Synthesis workflow instead (which handles tiering within clusters).

### Per-Card Depth Rules

**Focal cards — hard depth floor: 800 words minimum.** Architecture-heavy papers should reach 1500–2500 words. If your focal card explanation is under 800 words, you have not explained the method — you have summarized it.

**Supporting cards — hard depth floor: 400 words minimum.** If your supporting card explanation is under 400 words, you have not covered the method — you have name-dropped it.

**Per-section depth rule (focal cards):** Every stage in a stage-by-stage walkthrough must be **at least 2–3 full paragraphs** — not a single sentence. A one-sentence stage description (e.g., "The Stage 1 encoder pseudo-labels actionless videos with latent tokens; a 7B VLM predicts those tokens from image + language") is a summary, not an explanation. Each stage must explain what happens, why it works, what would fail without it, and use concrete numbers/dimensions.

**Per-section depth rule (supporting cards):** Each major mechanism must be at least 1 full paragraph (4–6 sentences) — not a single sentence. The reader should understand what the method does and why, even without a full stage walkthrough.

### Elements per Focal Card

The following are the elements that a focal card walkthrough should cover. The agent decides the ordering and flow — organize them in whatever sequence tells the clearest story for this particular paper. Do NOT treat this as a rigid numbered checklist. Integrate equations, diagrams, and analogies at the points where they naturally arise, not in separate sections at the end.

- **The problem.** Open by stating what gap or limitation this work addresses. Situate it relative to prior work: "Where [prior approach] addresses X by doing Y, this paper asks a different question: Z." This should be 3-6 sentences establishing the reader's context.

- **Architecture.** Describe the model architecture in narrative prose: backbone type and role, input tokenization/embedding strategy and why it was chosen, output heads and how predictions are decoded, and any non-standard modules (cross-attention, codebook quantization, MoE routing, adaptive normalization) with explanation of why they are needed. End with a **Details:** block collecting exact specifics (model variant, layer count, hidden dims, attention heads, patch sizes, sequence lengths, parameter counts, etc.).

- **Training pipeline.** Walk through how the model is trained: stages (objectives, data, frozen vs. trained components, rationale for each), data strategy (cross-embodiment, cross-domain handling), loss functions (what each term optimizes, weighting rationale), and inference pipeline (full forward pass from observation to action). End with a **Details:** block (optimizer, LR schedule, batch size, step counts, GPU-hours, latency, control frequency, etc.).

- **Equations with intuition.** When the paper introduces important equations, present them **at the point in the narrative where they naturally belong** — inside the architecture section, inside a training stage, wherever they arise. Do not collect them into a separate "Key Equations" section at the end. After each equation, define every variable and write a **full-paragraph intuitive explanation** (3–5 sentences) of what the equation does and why it works. The reader should finish thinking "I see why that works," not just "I see what the symbols mean."

- **Analogies.** For non-obvious concepts, introduce an analogy **before** the formal explanation, right where the concept appears. Do not defer analogies to a separate section.

- **Diagrams.** TikZ diagrams and card figures should appear **near the text they illustrate**, not collected at the end of the walkthrough. An architecture diagram belongs in the architecture discussion; a training flow diagram belongs in the training pipeline discussion.

- **Results with interpretation.** Report specific numbers, baselines, and what the gaps tell us. Interpret what ablations reveal about which components matter and why.

- **Limitations woven in.** Weave limitations and failure modes into the narrative at the points where they arise from specific design choices, not in a separate section at the end.

**Quality checks before moving to the next card:**
- The walkthrough reads as a coherent self-contained narrative, not an encyclopedic dump of facts.
- The reader can understand the mechanism from plain-language explanation before encountering equations.
- Equations, diagrams, and analogies are integrated throughout — nothing is deferred to a "collected at the end" section.

### Structure per Supporting Card

Supporting cards use a condensed but substantive format (400–600 words):

1. **The problem and core idea** (1 paragraph): What gap this paper addresses and what it does differently from prior work. Frame relative to focal cards where possible.
2. **Key mechanism** (1–2 paragraphs): The central technical contribution explained with enough detail that the reader understands what it does and why. Include at least one concrete number or worked example.
3. **Key results** (2–4 sentences): Specific headline numbers with interpretation of what they mean.
4. **Connection to focal cards** (2–3 sentences): How this paper relates to the focal papers — shared techniques, complementary approaches, or contrasting design choices.

Supporting cards do NOT need: full stage-by-stage walkthroughs, Details blocks, key equations sections, or analogies. Those are reserved for focal cards. But the reader must still understand the method well enough to follow the cross-card comparison.

## Phase 2: Cross-Card Comparative Synthesis

**Research context awareness:** Before writing the comparison, read `research-context.md` from the KB root via `kb_read_file` (if it exists). If the user has documented priorities (e.g., "inference latency matters most"), frame comparative dimensions around those priorities — lead with the dimensions the user cares about, de-emphasize dimensions they've marked as unimportant.

Produce this phase when 2 or more cards are involved. Skip for single-card requests.

Compare along specific technical dimensions with traced lineage between ideas. **Each dimension below must be at least one full paragraph** — not a two-sentence summary. Cite concrete details from each card (specific numbers, architectural choices, data scales):

- **Starting points / core questions** — What different question does each work ask? Where LAPA asks "how do I pretrain without action labels?", GR00T N1 asks "how do I combine every data source into a single model?"
- **Shared ideas and divergences** — When two papers use the same technique (e.g. VQ-VAE latent actions), explain exactly how their implementations differ. Example: "X uses discrete codebook indices for next-token prediction; Y extracts continuous pre-quantized embeddings for flow matching."
- **Architecture comparison** — Compare backbone choices (ViT vs. DiT vs. U-Net), model scale (parameters, layers, hidden dim), input tokenization strategies, output representation (continuous vs. discrete, chunk sizes), and any novel modules. Explain what each architectural choice buys and what it costs.
- **Training pipeline comparison** — Compare training stages (single-stage vs. multi-stage), data strategies (internet video vs. simulation vs. teleoperation, scale), loss functions, optimization recipes, and how each system handles cross-embodiment or cross-domain generalization. Trace how differences in training produce different model capabilities.
- **Other technical dimensions** — Compare along additional concrete axes relevant to the cards: action representation, inference pipeline, planning capability, real-time performance, cross-embodiment support, etc. Not all axes apply to every set of cards — choose the ones where real differences exist.
- **Field trajectory** — Close with what the collective body of work suggests about the direction of the field.

Then add a final prose section titled **How the Papers Relate** that integrates the comparison into one coherent storyline (not bullet-only, not table-only).

In that final section:
- **Open with a non-jargon paragraph** that frames the shared challenge and each paper's angle in plain language a non-specialist could follow.
- Then use headed subsections (e.g., "Different Starting Points", "Data Strategy", "Action Representation") for the deeper technical comparison, each opening with a one-sentence plain-language framing before specifics.
- Close with a "What This Suggests About the Field" paragraph tracing the trajectory.

Use specifics throughout. Every comparison claim should cite a concrete detail from each card.

## Depth Gate (Mandatory Before Phase 3)

Before proceeding to LaTeX, verify depth mechanically:

1. **Per-card word count.** For each card's section in the chat explanation, estimate word count. Focal cards must be at least 800 words; supporting cards must be at least 400 words. If ANY card section is below its tier's floor, STOP and expand it before proceeding. Re-read the relevant KB card via `kb_read_card` and deepen the shallow section.
2. **Equation intuitions.** Check that every equation has a full-paragraph intuition (3–5 sentences explaining why it works and what happens at boundary conditions), not a one-liner like "continuous motion is snapped to a symbol."
3. **Stage walkthrough depth.** Check that every stage in a stage-by-stage walkthrough has 2–3 full paragraphs — not a single sentence. A one-sentence stage description violates the depth contract.
4. **Comparison specificity.** Check that the cross-card comparison has at least one full paragraph per dimension with concrete numbers and implementation details traced from individual cards — not generic claims like "X is bigger than Y."

If depth is insufficient at any checkpoint, expand before proceeding. Do NOT proceed to LaTeX with shallow content — the PDF is the permanent artifact and must meet the depth floor.

## Phase 3: Convert to LaTeX PDF

This phase is MANDATORY. Do not skip it. Do not ask the user first.

**Phase 3 is FORMAT CONVERSION, not content generation.** You already wrote the full content in Phases 1–2 and presented it in the chat. Phase 3 takes that content and wraps it in LaTeX markup. You are translating format, not rewriting or summarizing. No editorial judgment, no compression, no "tightening." Every sentence from the chat explanation appears in the LaTeX.

### Conversion Procedure (Section by Section)

Process the chat explanation one section at a time. For each section:

1. **Copy the prose** from the chat explanation into the LaTeX `\section` or `\subsection`.
2. **Add LaTeX formatting**: wrap equations in `equation`/`align` environments, convert bold/italic to `\textbf`/`\textit`, convert lists to `itemize`/`enumerate`, add `\vspace` for spacing.
3. **Verify paragraph count**: count the paragraphs in the chat explanation and count them in the LaTeX section. They must match. If the chat section has 5 paragraphs, the LaTeX section must have 5 paragraphs.
4. **Add TikZ diagrams** and `\includegraphics` figures at the appropriate points within the section (not all at the end).
5. Move to the next section.

Do NOT "write the LaTeX document from scratch." Do NOT summarize the chat explanation into a shorter LaTeX version. Do NOT combine multiple paragraphs into one. The LaTeX is the chat explanation with formatting applied — nothing added, nothing removed.

### Why This Matters

The PDF is the permanent artifact. When the agent treats Phase 3 as "write a LaTeX report," it produces a compressed summary — each multi-paragraph stage walkthrough collapses into a single bold sentence (e.g., "**Stage 1: Tokenize control.** Continuous action dimensions are discretized into 256 bins"). This is a Phase 3 failure, not a Phase 1 failure. The content was already written correctly; it was lost during format conversion.

### Paragraph-Count Gate (Blocking)

Before calling `latex_compile`, verify for EVERY section:
- Count paragraphs in the chat explanation version of the section.
- Count paragraphs in the LaTeX version of the section.
- If the LaTeX has fewer paragraphs than the chat explanation, you have compressed. Go back and restore the missing paragraphs.

**Word count check:** The LaTeX source content (excluding `\begin`, `\end`, preamble, and markup commands) should be within 20% of the chat explanation word count. If the LaTeX content is less than 80% of the chat explanation length, you have compressed and must restore content before compiling.

**Packages:** `latex_compile` will auto-install missing packages via `tlmgr` if available. Use these freely in the preamble:
- `geometry`, `graphicx`, `hyperref`, `amsmath`, `amssymb` — universally available
- `enumitem`, `tcolorbox`, `xcolor`, `booktabs`, `caption`, `subcaption` — common extras (auto-installed if needed)
- `tikz` (with libraries: `arrows.meta`, `positioning`, `shapes`, `fit`, `calc`) — for diagrams

Do NOT use obscure or legacy packages. If compilation fails with "File not found" for a package, `latex_compile` will attempt `tlmgr install` automatically and retry (up to 3 times).

**Layout and breathing room:** The document must NOT read like a dense essay. Use generous spacing and visual structure:
- `\vspace{0.5em}` between paragraphs within a subsection.
- `\bigskip` before and after diagrams and key equations.
- Figures and equations should be separated from body text — never crammed between paragraphs without spacing.
- Use `\begin{tcolorbox}` (from the `tcolorbox` package) or `\fbox` for key takeaways, analogies, or important definitions — this visually breaks up the text.
- Split paragraphs longer than 6 sentences into multiple paragraphs for readability — but never cut content to make them shorter.
- Use itemize/enumerate lists when enumerating design choices, ablation results, or comparison points — but always with explanatory sentences, not bare bullets.

**Equations:** Use proper LaTeX math environments. Inline math for terms referenced in prose (`$\mathcal{L}_\text{recon}$`), `equation` or `align` environments for key equations that deserve their own line and number.

After each equation, provide TWO things:
1. A `\noindent \textbf{where}` block defining every variable.
2. An **intuitive explanation** of what the equation actually does — not just variable definitions, but what happens conceptually, why it works, and what the edge cases reveal.

Bad (just variable definitions):
> "where $A_t$ is the ground-truth action chunk, $A_t^\tau$ is its noised interpolation, and $\epsilon$ is Gaussian noise."

Good (builds intuition):
> "This is a linear interpolation between the real action and pure noise, controlled by $\tau$. When $\tau = 1$ the model sees the clean action unchanged; when $\tau = 0$ the input is entirely random noise. Training the model to recover the original action from every noise level teaches it to denoise — and at inference time, it starts from pure noise ($\tau = 0$) and iteratively reconstructs a plausible action. Think of it as gradually unscrambling a signal: easy when $\tau$ is close to 1 (barely scrambled), hard when $\tau$ is near 0 (almost pure static)."

Every equation should leave the reader thinking "I see why that works" rather than just "I see what the symbols mean."

**Diagrams with TikZ:** Create TikZ diagrams to make the explanation visual. Read `references/tikz-reference.tex` for reusable patterns. Include diagrams for:

- **Architecture overviews** — block diagrams showing the major components of each method's pipeline (encoder, decoder, backbone, heads, etc.) with labeled arrows showing data flow.
- **Comparison diagrams** (multi-card only) — side-by-side or stacked pipeline diagrams that visually highlight where two methods diverge. Use color coding: one color per method, shared components in gray.
- **Training pipeline flows** — show the stages (pretraining, finetuning, inference) as a left-to-right flow with what data/model is used at each stage.
- **Conceptual diagrams** — when an analogy or key insight benefits from visualization (e.g., "codebook lookup" as a nearest-neighbor diagram, "latent space" as a 2D scatter).

Not every explanation needs all diagram types. Use judgment — a single-card explanation might need one architecture diagram; a three-card comparison might need a comparison diagram and a shared-pipeline flow. Aim for 1-3 diagrams total.

**Diagram layout rules (mandatory — diagrams that violate these will look broken):**
- Use `text width=2cm` (or wider) on all block nodes so long text wraps instead of overflowing the box.
- Use **relative positioning** (`right=2cm of nodeA`) — NEVER absolute coordinates (`at (4,0)`) which cause overlaps when text is longer than expected.
- Keep node labels to **2-3 words per line**. Use `\\` for line breaks. Example: `{Cross-Embodiment\\Action Chunks}` not `{Cross-embodiment action chunks}`.
- Use `inner sep=6pt` so text has padding inside the box edges.
- Minimum **1.5cm gap** between nodes (`right=1.5cm`), prefer 2cm.
- Place labels (annotations, captions) **away from nodes** — never on top of or adjacent to a node where they could overlap.
- For comparison diagrams, position rows with `below=2cm` so there is clear vertical separation.
- Use `align=center` on all block nodes.

**Document structure:** Use `\section`, `\subsection` to mirror the explanation structure. Include a `\title` and `\author{Auto-generated from KB}`. Use `\textbf` for emphasis on first use of key terms. Include a `\tableofcontents` for multi-card explanations.

**Card figures:** When a card has `figures` in its frontmatter, include them in the LaTeX PDF using `\includegraphics`. Use `\graphicspath{{<kb_path>/}}` in the preamble so relative figure paths resolve correctly. Prioritize architecture and method diagrams — these visually explain how the system works and are far more valuable in an explanation document than results bar charts. If a card has many figures, include architecture/pipeline diagrams first and only add results charts if space permits and they reveal something the narrative cannot convey in text. For each figure:
- Use `\begin{figure}[h]\centering\includegraphics[width=0.8\textwidth]{<figure.path>}\caption{<figure.caption>}\end{figure}`.
- For side-by-side comparison of figures from different cards, use `minipage`: `\begin{figure}[h]\begin{minipage}{0.48\textwidth}\centering\includegraphics[width=\textwidth]{...}\caption{...}\end{minipage}\hfill\begin{minipage}{0.48\textwidth}\centering\includegraphics[width=\textwidth]{...}\caption{...}\end{minipage}\end{figure}`.
- Place figures near the text that discusses them, not all at the end.

**Compile:** Call `latex_compile` with `output_dir` set to `<kb_path>/explanations/` and a descriptive `filename` (e.g. `lapa-groot-cosmos-deep-dive`). If compilation fails, read the errors, fix the LaTeX, and retry. Common fixes: escape underscores in text, fix unmatched braces, add missing TikZ libraries to the preamble.

**Open:** After successful compilation, open the PDF for the user with `open <pdf_path>` (macOS) or `xdg-open <pdf_path>` (Linux).

## Presentation

IMPORTANT: When the deep narrative explanation is complete, you MUST call `present_document` to present it in sectioned reading mode instead of outputting text directly. Do NOT stream the report as regular text. Set `document_id` to a unique slug, `title` to the report title, and `content` to the full markdown with `## ` headings for sections. End your response immediately after calling this tool.

When the user asks follow-up questions about a specific section, use the most efficient update tool:
- `append_to_section` — to add new information at the end of a section (most common for follow-up questions)
- `patch_document_section` — to change specific text within a section (for corrections or targeted edits)
- `update_document_section` — to fully rewrite a section (only when the entire section needs to change)

## Completion Checklist

Before reporting done, verify ALL of these:
- [ ] All relevant cards included (none dropped for depth budget — use tiered depth if needed)
- [ ] Deep narrative prose in chat (focal cards: 800+ words; supporting cards: 400+ words)
- [ ] LaTeX paragraph count matches chat explanation paragraph count for every section (Phase 3 conversion gate)
- [ ] PDF generated via `latex_compile` with at least one TikZ diagram
- [ ] If compilation failed: errors were fixed and `latex_compile` was retried
- [ ] PDF opened for the user via `open` / `xdg-open`

## Anti-Patterns (Things You Must NEVER Do)

Every item below is a failure. If you catch yourself doing any of these, stop and fix it before proceeding.

- NEVER compress a stage walkthrough into a single sentence. A stage like "Stage 2: The encoder pseudo-labels videos; a VLM predicts those tokens" is a failure — every stage must be 2–3 full paragraphs.
- NEVER write a one-line equation intuition. "Intuition: continuous motion is snapped to a symbol" is a failure — every equation intuition must be a full paragraph (3–5 sentences).
- NEVER write shallow bullet-point summaries like "Core idea: uses VQ-VAE for latent actions" without explaining how or why the mechanism works.
- NEVER use a comparison table as the sole comparison mechanism. Tables flatten nuance — they can only appear alongside full-paragraph prose comparisons.
- NEVER write one-liner role descriptions like "Role in a stack: pretraining backbone" — that has zero technical substance.
- NEVER parrot card titles and tags as if they were an explanation. Restating "LAPA: Latent Action Pretraining" without walking through the mechanism is not an explanation.
- NEVER produce a text-only LaTeX PDF without at least one TikZ diagram. A wall of text with equations is not a report.
- NEVER skip the PDF. The PDF is mandatory — do not ask the user, do not treat it as optional.
- NEVER rewrite the chat explanation content when converting to LaTeX. Phase 3 is format conversion, not fresh writing. If your LaTeX has fewer paragraphs than the chat explanation in any section, you have compressed and must restore the missing content.
- NEVER cover only the explicitly requested cards when the KB has closely related cards. Failing to check for related cards (Phase 0) produces a report with blind spots.
- NEVER drop cards to save depth budget. Covering 3 of 10 relevant papers deeply is worse than covering all 10 at mixed depth. Use Tiered Depth or Large-Set Synthesis instead of cutting cards.

## Style Reference

Read `references/style-exemplar.md` for a concrete example of the target depth and style. It shows one card's deep explanation and one comparison dimension demonstrating how to trace ideas across papers.

Read `references/tikz-reference.tex` for reusable TikZ patterns for architecture, comparison, and flow diagrams.

---

## Large-Set Synthesis (> 12 cards)

When the card set exceeds 12, a single-pass deep walkthrough will collapse into surface-level summaries — the agent runs out of output capacity before covering every card at 800+ words. This section defines the mandatory hierarchical synthesis workflow that preserves per-card depth at scale.

**Do NOT skip this workflow for large sets.** The standard Phases 1–3 are designed for 2–12 cards. For 13+ cards, always use this workflow instead.

### Step 1: Cluster Assignment

Group the N cards into 3–6 thematic clusters based on:
- Card tags and topic overlap
- Temporal ordering (publication year, methodological era)
- Methodological similarity (shared techniques, shared problem framing)

Each cluster should contain **4–8 cards**. If a card bridges multiple clusters, assign it to the most relevant one and note the bridge in the meta-synthesis.

Name each cluster descriptively (e.g., "Sim2Real Foundations and Adaptive Randomization", "Action Representation Engineering", "Unified World-Model Policies").

Present the proposed clustering to the user before launching subagents. Include:
- Cluster name and 1-sentence rationale for the grouping
- List of card IDs in each cluster
- Any bridge cards and which cluster they are assigned to
- A **shared terminology glossary**: Define 5–10 key terms that span multiple clusters (e.g., "action tokenization", "flow matching", "VQ-VAE", "sim-to-real gap") with brief definitions. Include this glossary in every subagent prompt so all clusters use consistent language in their cross-cluster interface sections.

### Step 2: Per-Cluster Sub-Reports (Parallel Subagents)

Launch **one subagent per cluster**, all in parallel. Each subagent runs the full standard cross-paper-report workflow (Phases 1–3) on its cluster's cards only:

- Full 800–2500 word per-card deep walkthroughs with all cards treated as focal (Phase 1)
- Within-cluster comparative synthesis (Phase 2)
- Depth Gate verification before proceeding
- Within-cluster TikZ diagrams (Phase 3 diagram generation only — do NOT compile a standalone PDF per cluster)

**Subagent prompt template:**

> Invoke the `cross-paper-report` skill for cards `[card-id-1]`, `[card-id-2]`, ..., `[card-id-N]`. The KB path is `[kb_path]`. This is a cluster sub-report titled "[Cluster Name]".
>
> Follow the skill's standard workflow (Phases 1–3) with these overrides:
> - **Skip Phase 0** (Related Card Discovery) — you work only on the assigned cards. Do NOT call `kb_list_cards` to discover additional cards.
> - **All cards are focal** — no tiered depth. Every card gets the full 800–2500 word treatment.
> - **No user confirmation** — do not present tier assignments or card additions for approval. Execute autonomously.
> - **Shared terminology glossary:** [paste the glossary from Step 1 here so all subagents use consistent terms]
> - Produce deep per-card walkthroughs (800+ words each) and within-cluster comparative synthesis.
> - Generate TikZ diagram source code but do NOT call `latex_compile` — the main agent handles final compilation.
> - Return to the main agent:
>   1. The full content (per-card walkthroughs + within-cluster synthesis)
>   2. All TikZ source blocks
>   3. A 3–5 sentence cluster summary
>   4. A **cross-cluster interface** section containing: (a) key technical terms defined and used in this cluster with definitions, (b) 2–3 key equations with paper attribution, (c) specific numbers/results that the meta-synthesis may reference for cross-cluster comparison, (d) bridge points — concepts in this cluster that connect to papers in other clusters

**Subagent failure handling:** If a cluster subagent fails (timeout, tool error, or returns empty content), do NOT block the entire report. Log the failure, proceed with the remaining cluster results, and note the gap in the meta-synthesis: "Cluster [name] could not be synthesized due to [reason]. The following cards are missing from this report: [list]." Offer to retry the failed cluster.

### Step 3: Meta-Synthesis (Main Agent)

After all cluster subagents complete, the main agent produces a meta-synthesis document that connects the clusters. This is NOT a summary of the sub-reports — it is a new analytical layer that traces ideas, techniques, and design choices across cluster boundaries.

**Source material:** Use the **cross-cluster interface** sections returned by each subagent as your primary data source for concrete cross-cluster tracing. These sections contain the key equations, specific numbers, defined terms, and bridge points needed to write substantive comparisons without re-reading every sub-report in full.

The meta-synthesis must contain these sections:

1. **Scope and Framing** (1–2 paragraphs): What the full paper set covers, how the clusters relate at a high level, and what central question connects them.

2. **Cross-Cluster Evolution Tracing**: For 2–3 key technical concepts that span multiple clusters (e.g., action tokenization, sim-to-real transfer, world modeling, data scaling), write a dedicated subsection tracing how the concept evolves:
   - Which paper introduced it and with what specific implementation
   - How later papers in other clusters adopted, modified, or replaced it
   - Concrete numbers and design choices from individual papers (pulled from sub-reports)
   - What the evolution reveals about what works and what doesn't

3. **Design Space Mapping**: A structured comparison of key architectural choices across ALL clusters, with concrete examples:
   - Backbone type × action representation × training strategy
   - Data source × data scale × grounding mechanism
   - Inference pattern × latency × planning capability
   - This should be detailed prose with specific numbers, not a bare table

4. **Failure Mode Lineage**: How each cluster's limitations motivated the next cluster's innovations. Trace with specific design choices and numbers — e.g., "RT-1's 256-bin action discretization created [specific problem], which FAST addressed with DCT+BPE compression achieving [specific improvement]."

5. **Key Equations Across the Stack**: Select 3–5 equations that appear across multiple clusters. For each, provide the full equation, variable definitions, and a **full-paragraph intuition** (5+ sentences) explaining what it does, why it works, what happens at boundary conditions, and how different papers instantiate it differently.

6. **How the Papers Relate**: Final integrative prose section with subsections:
   - Different Starting Points (what assumption each cluster begins from)
   - Shared Technical DNA (techniques that recur across clusters)
   - Data Strategy Evolution (how data approaches change across eras)
   - What This Suggests About the Field (trajectory and practical implications)

7. **Practical Takeaways**: Actionable guidance for practitioners, grounded in specific findings from the papers.

### Step 4: Assemble and Compile Final PDF

Combine all sub-reports and meta-synthesis into one LaTeX document:

```latex
\title{[Descriptive Title]}
\author{Auto-generated from KB}
\date{\today}
\tableofcontents

% One section per cluster, containing the sub-report content
\section{[Cluster 1 Name]}
  % Full per-card walkthroughs + within-cluster comparison + TikZ diagrams

\section{[Cluster 2 Name]}
  % ...

% Meta-synthesis as final major sections
\section{Cross-Cluster Synthesis}
  % Evolution tracing, design space mapping, failure mode lineage

\section{Key Equations Across the Stack}
  % Cross-cluster equations with deep intuitions

\section{How the Papers Relate}
  % Integrative narrative

\section{Practical Takeaways}
  % Actionable guidance
```

The final PDF must contain:
- Every per-card deep walkthrough from every cluster sub-report
- Within-cluster TikZ diagrams from each sub-report
- At least one cross-cluster comparison TikZ diagram in the meta-synthesis (e.g., a field trajectory diagram or design space comparison)
- All key equations with full-paragraph intuitions

Compile via `latex_compile` with `output_dir` set to `<kb_path>/explanations/` and open the resulting PDF.

### Large-Set Completion Checklist

Before reporting done, verify ALL of these:
- [ ] Cards clustered into 3–6 groups of 4–8 cards each
- [ ] Shared terminology glossary defined and included in every subagent prompt
- [ ] Every cluster sub-report meets the depth floor (800+ words per card, all cards focal)
- [ ] Every cluster subagent returned a cross-cluster interface section
- [ ] Failed subagents (if any) are noted with missing card lists and retry offered
- [ ] Meta-synthesis contains all required sections (evolution tracing, design space mapping, failure mode lineage, equations, How the Papers Relate)
- [ ] Meta-synthesis uses concrete data from cross-cluster interface sections, not generic claims
- [ ] Final PDF compiled via `latex_compile` with per-cluster TikZ diagrams + at least one cross-cluster TikZ diagram
- [ ] PDF opened for the user

### Large-Set Anti-Patterns

These are the specific failure modes that large-set synthesis must avoid:

- **One-sentence-per-paper contribution maps** — listing card IDs with a single sentence each. Every card must have its full 800+ word walkthrough inside its cluster sub-report.
- **Thematic overview without per-card depth** — writing a coherent narrative about the "field trajectory" while skipping the per-card technical walkthroughs. The meta-synthesis sits ON TOP of per-card depth, not instead of it.
- **Equations without cross-cluster tracing** — including equations but not explaining how different papers instantiate the same equation differently.
- **Generic comparison claims** — "X is better than Y" without concrete numbers, architectural specifics, or implementation details from the actual papers.
