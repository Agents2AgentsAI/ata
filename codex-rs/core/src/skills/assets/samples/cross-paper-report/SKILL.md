---
name: cross-paper-report
description: Generate integrated cross-paper explanations and final markdown/PDF reports from KB cards. Use when a user asks to explain, compare, synthesize, understand, or deep-dive into one or more knowledge base cards, or asks how cards relate or work together.
---

# Cross-Paper Report

You MUST produce three deliverables — no exceptions, no shortcuts:

1. **Deep narrative explanation** in the chat (800-2500 words per card)
2. **Markdown file** saved to the KB via `kb_write_file`
3. **LaTeX PDF** compiled via `latex_compile` with TikZ diagrams

A short summary is NEVER acceptable. This task is NOT complete until all three deliverables exist. Do NOT ask the user whether they want a PDF — always produce it.

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
- Keep paragraphs short and use explicit transitions (`Now`, `Next`, `At inference`, `Why this matters`).
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

## Phase 1: Deep Technical Explanation of Each Card

Read all requested cards via `kb_read_card` (or `kb_list_cards` + `kb_read_card` when the user says "all" or refers to a broad set).

For EACH card, write a deep technical walkthrough in **narrative prose**. This phase applies whether the user asks about one card or many.

**Hard depth floor: 800 words minimum per card.** Architecture-heavy papers should reach 1500-2500 words. If your card explanation is under 800 words, you have not explained the method — you have summarized it, which violates this skill's contract.

**Per-section depth rule:** Every stage in a stage-by-stage walkthrough must be **at least 2-3 full paragraphs** — not a single sentence. A one-sentence stage description (e.g., "The Stage 1 encoder pseudo-labels actionless videos with latent tokens; a 7B VLM predicts those tokens from image + language") is a summary, not an explanation. Each stage must explain what happens, why it works, what would fail without it, and use concrete numbers/dimensions.

### Structure per Card

1. **The problem.** Open by stating what gap or limitation this work addresses. Situate it relative to prior work: "Where [prior approach] addresses X by doing Y, this paper asks a different question: Z." This should be 3-6 sentences establishing the reader's context.

2. **Architecture deep dive.** Describe the full model architecture in narrative prose, focusing on what each component does and why:
   - **Backbone / encoder / decoder** — What is the core network type and what role does it play? Explain the architecture conceptually (e.g., "a diffusion Transformer that denoises latent sequences step by step").
   - **Input representation** — How are raw inputs tokenized or embedded before entering the model? Explain the encoding strategy and why it was chosen.
   - **Output heads** — What does the model predict and how are outputs decoded back to the action/control space? Explain the design rationale.
   - **Key modules** — Describe any non-standard components: cross-attention between modalities, adaptive layer norm, codebook quantization, mixture-of-experts routing, etc. For each, explain the mechanism and why it's needed.
   - End with a **Details:** block collecting exact specifics: model variant name, layer count, hidden dims, attention heads, patch sizes, resolution, token dimensions, sequence lengths, positional encoding type, output dimensions, chunk sizes, prediction horizons, parameter counts.

3. **Training pipeline.** Describe how the model is trained end-to-end:
   - **Stages** — Is training single-stage or multi-stage? For each stage, explain the objective, what data is used and why, which components are frozen vs. trained, and the rationale for each choice.
   - **Data strategy** — What data is used at each stage and why? How is cross-embodiment or cross-domain data handled? Explain the strategy conceptually.
   - **Loss functions** — Explain every loss term: what it optimizes and why. For compound losses, explain the weighting rationale. Include mathematical form in the Key Equations section.
   - **Inference pipeline** — How does the trained model run at test time? Describe the full forward pass from raw observation to executed action, including iterative processes and their purpose.
   - End with a **Details:** block collecting: optimizer name, learning rate schedule, batch size, gradient clipping, mixed precision, distributed strategy, step/epoch counts, GPU-hours, noise schedule parameters, EMA decay, codebook sizes, temperature values, action chunk lengths, latency numbers, control frequency.

4. **Key equations.** Reproduce important equations and define every variable. Do not skip terms or hand-wave over notation. After defining variables, write a **full-paragraph intuitive explanation** (3-5 sentences minimum) of what the equation does and why it works — not a one-liner like "continuous motion is snapped to a symbol." The reader should finish the paragraph thinking "I see why that works," not just "I see what the symbols mean."

5. **Analogies.** For non-obvious concepts, provide an analogy that builds intuition before the formal explanation. Example: "Think of this as building an action dictionary from scratch."

6. **Results with interpretation.** Report specific numbers, baselines, and what the gaps tell us. Do not merely restate tables — interpret what the ablations reveal about which components matter and why. Pay special attention to ablations that isolate individual architectural or training decisions.

7. **Limitations woven in.** Weave limitations and failure modes into the narrative at the point where they arise from specific design choices, rather than listing them separately at the end.

8. **Walkthrough readability check.** Ensure the final card explanation reads as a coherent self-contained walkthrough, not an encyclopedic dump of facts.

9. **Basics-first check.** Confirm the reader can understand the mechanism from plain-language pipeline explanation before reading equations.

## Phase 2: Cross-Card Comparative Synthesis

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

## Phase 3: Save Markdown

Write the full explanation (Phase 1 + Phase 2) using `kb_write_file` with path `explanations/<descriptive-name>.md`. The `<descriptive-name>` should reflect the content (e.g. `lapa-groot-cosmos-deep-dive.md`, `diffusion-policy-methods-compared.md`). Parent directories are created automatically.

Also present the full explanation in the chat response so the user sees it immediately.

## Phase 4: Generate LaTeX PDF

This phase is MANDATORY. Do not skip it. Do not ask the user first.

Call `latex_compile` to produce a typeset PDF. The PDF is the primary deliverable — it should look like a well-typeset technical report, not a wall of text.

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
- Aim for short paragraphs (3-5 sentences). If a paragraph exceeds 6 sentences, split it.
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

**Compile:** Call `latex_compile` with `output_dir` set to `<kb_path>/explanations/` and a `filename` matching the markdown name (without extension). If compilation fails, read the errors, fix the LaTeX, and retry. Common fixes: escape underscores in text, fix unmatched braces, add missing TikZ libraries to the preamble.

**Open:** After successful compilation, open the PDF for the user with `open <pdf_path>` (macOS) or `xdg-open <pdf_path>` (Linux).

## Completion Checklist

Before reporting done, verify ALL of these:
- [ ] Deep narrative prose in chat (800 words minimum per card — if under 800 words, expand before proceeding)
- [ ] Markdown saved via `kb_write_file` to `explanations/<name>.md`
- [ ] PDF generated via `latex_compile` with at least one TikZ diagram
- [ ] If compilation failed: errors were fixed and `latex_compile` was retried
- [ ] PDF opened for the user via `open` / `xdg-open`

## Anti-Patterns

Do NOT produce any of the following as the primary output format:

- **Skeletal stage walkthroughs** — compressing each stage to a single sentence (e.g., "Stage 2: The encoder pseudo-labels videos; a VLM predicts those tokens"). Each stage must be 2-3 paragraphs explaining what, why, and how with concrete numbers.
- **One-line equation intuitions** — writing "Intuition: continuous motion is snapped to a symbol" after an equation. Intuition must be a full paragraph (3-5 sentences) that explains why the equation works and what happens at the boundary conditions.
- **Shallow bullet-point summaries** — "Core idea: uses VQ-VAE for latent actions" with no elaboration of how or why
- **Comparison tables** as the sole comparison mechanism — tables flatten technical nuance into cells
- **One-liner role descriptions** — "Role in a stack: pretraining backbone" without technical substance
- **Title/tag parroting** — restating card titles and tags without walking through the actual mechanism
- **Text-only LaTeX** — if producing a PDF, it must include at least one TikZ diagram; a wall of text with equations is not sufficient
- **Skipping the PDF** — the PDF is mandatory, not optional

These formats are acceptable only as secondary navigation aids alongside the deep narrative prose.

## Style Reference

Read `references/style-exemplar.md` for a concrete example of the target depth and style. It shows one card's deep explanation and one comparison dimension demonstrating how to trace ideas across papers.

Read `references/tikz-reference.tex` for reusable TikZ patterns for architecture, comparison, and flow diagrams.
