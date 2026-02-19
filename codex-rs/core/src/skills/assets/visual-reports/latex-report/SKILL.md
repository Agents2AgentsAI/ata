---
name: latex-report
description: Convert an existing cross-paper-report narrative into a LaTeX PDF with TikZ diagrams. Use after generating a narrative via cross-paper-report when the user wants a compiled PDF.
metadata:
  short-description: Convert narrative reports to LaTeX PDF
---

# LaTeX Report

Convert an existing cross-paper-report narrative into a professionally formatted LaTeX PDF with TikZ diagrams. This skill is FORMAT CONVERSION, not content generation — it takes content already written via `$cross-paper-report` and wraps it in LaTeX markup.

## Prerequisites

Call `kb_status` first. The response includes `kb_path` — use that value wherever this document says `<kb_path>`.

A deep narrative explanation must already exist (produced by `$cross-paper-report`). If no narrative exists yet, tell the user to run `$cross-paper-report` first.

## Conversion Procedure (Section by Section)

Process the narrative explanation one section at a time. For each section:

1. **Copy the prose** from the narrative into the LaTeX `\section` or `\subsection`.
2. **Add LaTeX formatting**: wrap equations in `equation`/`align` environments, convert bold/italic to `\textbf`/`\textit`, convert lists to `itemize`/`enumerate`, add `\vspace` for spacing.
3. **Verify paragraph count**: count the paragraphs in the narrative and count them in the LaTeX section. They must match. If the narrative has 5 paragraphs, the LaTeX section must have 5 paragraphs.
4. **Add TikZ diagrams** and `\includegraphics` figures at the appropriate points within the section (not all at the end).
5. Move to the next section.

Do NOT "write the LaTeX document from scratch." Do NOT summarize the narrative into a shorter LaTeX version. Do NOT combine multiple paragraphs into one. The LaTeX is the narrative with formatting applied — nothing added, nothing removed.

### Why This Matters

The PDF is the permanent artifact. When the agent treats this as "write a LaTeX report," it produces a compressed summary — each multi-paragraph stage walkthrough collapses into a single bold sentence (e.g., "**Stage 1: Tokenize control.** Continuous action dimensions are discretized into 256 bins"). This is a conversion failure, not a content failure. The content was already written correctly; it was lost during format conversion.

### Paragraph-Count Gate (Blocking)

Before calling `latex_compile`, verify for EVERY section:
- Count paragraphs in the narrative version of the section.
- Count paragraphs in the LaTeX version of the section.
- If the LaTeX has fewer paragraphs than the narrative, you have compressed. Go back and restore the missing paragraphs.

**Word count check:** The LaTeX source content (excluding `\begin`, `\end`, preamble, and markup commands) should be within 20% of the narrative word count. If the LaTeX content is less than 80% of the narrative length, you have compressed and must restore content before compiling.

## Packages

`latex_compile` will auto-install missing packages via `tlmgr` if available. Use these freely in the preamble:
- `geometry`, `graphicx`, `hyperref`, `amsmath`, `amssymb` — universally available
- `enumitem`, `tcolorbox`, `xcolor`, `booktabs`, `caption`, `subcaption` — common extras (auto-installed if needed)
- `tikz` (with libraries: `arrows.meta`, `positioning`, `shapes`, `fit`, `calc`) — for diagrams

Do NOT use obscure or legacy packages. If compilation fails with "File not found" for a package, `latex_compile` will attempt `tlmgr install` automatically and retry (up to 3 times).

## Layout and Breathing Room

The document must NOT read like a dense essay. Use generous spacing and visual structure:
- `\vspace{0.5em}` between paragraphs within a subsection.
- `\bigskip` before and after diagrams and key equations.
- Figures and equations should be separated from body text — never crammed between paragraphs without spacing.
- Use `\begin{tcolorbox}` (from the `tcolorbox` package) or `\fbox` for key takeaways, analogies, or important definitions — this visually breaks up the text.
- Split paragraphs longer than 6 sentences into multiple paragraphs for readability — but never cut content to make them shorter.
- Use itemize/enumerate lists when enumerating design choices, ablation results, or comparison points — but always with explanatory sentences, not bare bullets.

## Equations

Use proper LaTeX math environments. Inline math for terms referenced in prose (`$\mathcal{L}_\text{recon}$`), `equation` or `align` environments for key equations that deserve their own line and number.

After each equation, provide TWO things:
1. A `\noindent \textbf{where}` block defining every variable.
2. An **intuitive explanation** of what the equation actually does — not just variable definitions, but what happens conceptually, why it works, and what the edge cases reveal.

Bad (just variable definitions):
> "where $A_t$ is the ground-truth action chunk, $A_t^\tau$ is its noised interpolation, and $\epsilon$ is Gaussian noise."

Good (builds intuition):
> "This is a linear interpolation between the real action and pure noise, controlled by $\tau$. When $\tau = 1$ the model sees the clean action unchanged; when $\tau = 0$ the input is entirely random noise. Training the model to recover the original action from every noise level teaches it to denoise — and at inference time, it starts from pure noise ($\tau = 0$) and iteratively reconstructs a plausible action. Think of it as gradually unscrambling a signal: easy when $\tau$ is close to 1 (barely scrambled), hard when $\tau$ is near 0 (almost pure static)."

Every equation should leave the reader thinking "I see why that works" rather than just "I see what the symbols mean."

## Diagrams with TikZ

Create TikZ diagrams to make the explanation visual. Read `references/tikz-reference.tex` for reusable patterns. Include diagrams for:

- **Architecture overviews** — block diagrams showing the major components of each method's pipeline (encoder, decoder, backbone, heads, etc.) with labeled arrows showing data flow.
- **Comparison diagrams** (multi-card only) — side-by-side or stacked pipeline diagrams that visually highlight where two methods diverge. Use color coding: one color per method, shared components in gray.
- **Training pipeline flows** — show the stages (pretraining, finetuning, inference) as a left-to-right flow with what data/model is used at each stage.
- **Conceptual diagrams** — when an analogy or key insight benefits from visualization (e.g., "codebook lookup" as a nearest-neighbor diagram, "latent space" as a 2D scatter).

Not every explanation needs all diagram types. Use judgment — a single-card explanation might need one architecture diagram; a three-card comparison might need a comparison diagram and a shared-pipeline flow. Aim for 1-3 diagrams total.

### Diagram Layout Rules (Mandatory)

Diagrams that violate these will look broken:
- Use `text width=2cm` (or wider) on all block nodes so long text wraps instead of overflowing the box.
- Use **relative positioning** (`right=2cm of nodeA`) — NEVER absolute coordinates (`at (4,0)`) which cause overlaps when text is longer than expected.
- Keep node labels to **2-3 words per line**. Use `\\` for line breaks. Example: `{Cross-Embodiment\\Action Chunks}` not `{Cross-embodiment action chunks}`.
- Use `inner sep=6pt` so text has padding inside the box edges.
- Minimum **1.5cm gap** between nodes (`right=1.5cm`), prefer 2cm.
- Place labels (annotations, captions) **away from nodes** — never on top of or adjacent to a node where they could overlap.
- For comparison diagrams, position rows with `below=2cm` so there is clear vertical separation.
- Use `align=center` on all block nodes.

## Document Structure

Use `\section`, `\subsection` to mirror the explanation structure. Include a `\title` and `\author{Auto-generated from KB}`. Use `\textbf` for emphasis on first use of key terms. Include a `\tableofcontents` for multi-card explanations.

## Card Figures

When a card has `figures` in its frontmatter, include them in the LaTeX PDF using `\includegraphics`. Use `\graphicspath{{<kb_path>/}}` in the preamble so relative figure paths resolve correctly. Prioritize architecture and method diagrams — these visually explain how the system works and are far more valuable in an explanation document than results bar charts. If a card has many figures, include architecture/pipeline diagrams first and only add results charts if space permits and they reveal something the narrative cannot convey in text. For each figure:
- Use `\begin{figure}[h]\centering\includegraphics[width=0.8\textwidth]{<figure.path>}\caption{<figure.caption>}\end{figure}`.
- For side-by-side comparison of figures from different cards, use `minipage`: `\begin{figure}[h]\begin{minipage}{0.48\textwidth}\centering\includegraphics[width=\textwidth]{...}\caption{...}\end{minipage}\hfill\begin{minipage}{0.48\textwidth}\centering\includegraphics[width=\textwidth]{...}\caption{...}\end{minipage}\end{figure}`.
- Place figures near the text that discusses them, not all at the end.

## Compile and Open

Call `latex_compile` with `output_dir` set to `<kb_path>/explanations/` and a descriptive `filename` (e.g. `lapa-groot-cosmos-deep-dive`). If compilation fails, read the errors, fix the LaTeX, and retry. Common fixes: escape underscores in text, fix unmatched braces, add missing TikZ libraries to the preamble.

After successful compilation, open the PDF for the user with `open <pdf_path>` (macOS) or `xdg-open <pdf_path>` (Linux).

## Style Reference

Read `references/style-exemplar.md` for a concrete example of the target depth and style. It shows one card's deep explanation and one comparison dimension demonstrating how to trace ideas across papers.

Read `references/tikz-reference.tex` for reusable TikZ patterns for architecture, comparison, and flow diagrams.

## Completion Checklist

Before reporting done, verify ALL of these:
- [ ] LaTeX paragraph count matches narrative paragraph count for every section
- [ ] At least one TikZ diagram included
- [ ] PDF generated via `latex_compile`
- [ ] If compilation failed: errors were fixed and `latex_compile` was retried
- [ ] PDF opened for the user via `open` / `xdg-open`

## Anti-Patterns

- NEVER produce a text-only LaTeX PDF without at least one TikZ diagram. A wall of text with equations is not a report.
- NEVER rewrite the narrative content when converting to LaTeX. This is format conversion, not fresh writing. If your LaTeX has fewer paragraphs than the narrative in any section, you have compressed and must restore the missing content.
- NEVER skip the PDF. The PDF is the whole point of this skill.
