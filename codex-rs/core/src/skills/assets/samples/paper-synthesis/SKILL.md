---
name: paper-synthesis
description: Synthesize academic papers into structured summaries and pedagogical deep dives. Use when a user asks to explain, summarize, synthesize, or deep-dive into a research paper, or when given an arXiv URL, DOI, paper title, or Zotero reference to analyze.
metadata:
  short-description: Summarize and explain research papers
---

# Paper Synthesis

Produce two complementary outputs for any research paper: a **Structured Summary** for quick reference and a **Pedagogical Deep Dive** for understanding. Default to both unless the user requests only one.

## Execution: Use Subagents

**Always launch a subagent for each paper.** This is mandatory, not optional:

- **Single paper**: Launch one subagent. This keeps the full paper text out of the main conversation context, preventing context pollution.
- **Multiple papers**: Launch one subagent per paper, **in parallel**. This dramatically speeds up multi-paper requests.

### Subagent Prompt Construction

You MUST copy the **entire workflow** into each subagent prompt — not a summary, not just the Type 1/Type 2 sections. The subagent has no access to this skill file; it only knows what you put in the prompt. Include ALL of the following sections verbatim (or faithfully paraphrased with every detail preserved):

1. **Pre-Synthesis** — how to obtain the full paper (Path A or Path B depending on source)
2. **Type 1: Structured Summary** — all headers: Metadata, Architecture, Training Pipeline, Core Method, Key Results, What's Novel, Limitations
3. **Type 2: Pedagogical Deep Dive** — all subsections: Framing, Architecture Deep Dive, Training Pipeline, Method Walkthrough, Analogies, Results Interpretation, Emergent Behaviors and Limitations, Connections
4. **Figure Extraction** — the `pdf_extract_figures` phase with output_dir, figure selection criteria, and how to pass figures to `kb_write_card`
5. **KB Card Storage** — frontmatter format, card body structure (Summary, Architecture, Training Pipeline, Deep Dive, Key Equations, Connections), and `kb_write_card` instructions

If you omit any section, the subagent WILL skip it. Common mistake: forgetting figure extraction or the detailed architecture/training requirements.

Each subagent prompt must also include:
- The paper identifier (URL, DOI, arXiv ID, or Zotero item key)
- The KB path from `kb_status` (so the subagent can write cards and figure assets)
- For Zotero papers: the item key and any fulltext/notes already retrieved by the main agent

### Main Agent Role

The main agent's role is to:
1. Call `kb_status` to get `kb_path`
2. Resolve which papers to synthesize (search Zotero, resolve URLs, etc.)
3. Construct complete subagent prompts with the full workflow above
4. Launch subagents in parallel
5. Collect results and present a summary to the user
6. If multiple papers: produce a cross-paper comparative section after all subagents complete

## Pre-Synthesis: Obtain the Full Paper

Before synthesizing, always attempt to read the full paper text. Choose the path that matches the user's input:

### Path A: arXiv URL or DOI (default)

1. If given an arXiv `/abs/` URL, convert it to `/pdf/` (e.g. `https://arxiv.org/abs/2503.14734` becomes `https://arxiv.org/pdf/2503.14734`).
2. Use `attach_url_files` to fetch the PDF. If available, use `paper_get` to retrieve metadata (title, authors, abstract) as supplementary context.
3. If PDF fetch fails, fall back to the abstract from `paper_get` and note in output: "Based on abstract only; full text unavailable."
4. If neither source is available, clearly state this limitation upfront.

### Path B: Zotero (when user mentions Zotero, a collection, or their library)

1. Use `zotero_search` to find the paper(s) by title, author, or topic. Also call `zotero_get_collections` to check if a collection matches the topic — if so, use `zotero_get_collection_items` to retrieve its contents. If the user names a specific collection, use `zotero_get_collection_items` directly.
2. For each paper found, call `zotero_get_item` for full metadata (title, authors, year, DOI, tags).
3. Call `zotero_get_fulltext` to get the indexed full text. This is the primary source — it contains the complete paper content as indexed by Zotero.
4. Optionally call `zotero_get_notes` to retrieve the user's annotations and highlights — weave these into the synthesis where relevant (e.g. "The authors note X, which the reader flagged as particularly relevant because...").
5. If `zotero_get_fulltext` returns no content, fall back to Path A using the DOI or arXiv ID from the Zotero metadata.

When the user asks to analyze multiple papers from Zotero (e.g. "synthesize my Zotero collection on diffusion"), launch one subagent per paper in parallel. After all subagents complete, produce a cross-paper comparative section in the main context (same format as the kb-explain cross-card synthesis).

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

### Framing
Open by situating the paper relative to prior work: "Where [prior approach] addresses X by doing Y, this paper asks a different question: Z." Establish why the reader should care.

### Architecture Deep Dive
Describe the full model architecture in narrative prose. Cover the backbone (type, depth, width, attention configuration), how each input modality is tokenized and embedded (dimensions, sequence lengths, positional encoding), what the output heads predict (action chunks, flow vectors, discrete tokens) and how predictions are decoded to the control space. Describe any non-standard modules — cross-attention mechanisms, codebook quantization layers, mixture-of-experts routing, adaptive normalization — explaining the mechanism and why the simpler alternative would fail. State parameter counts and breakdown when available.

### Training Pipeline
Walk through every training stage in order. For each stage, explain the objective, what data is used (source, scale, modalities), which components are frozen vs. trained, and duration (steps, GPU-hours). Describe the loss functions with their mathematical form and any weighting or annealing schedules. Cover optimization details: optimizer, learning rate schedule (warmup, decay), batch size, gradient clipping, distributed strategy. Explain the inference pipeline end-to-end — from raw sensor observation through the model to executed action — including iterative processes (diffusion denoising steps, autoregressive decoding), latency, and control frequency.

### Method Walkthrough
Walk through the core method step by step with specific numbers inline. For each design choice, explain WHY it was made — what would go wrong with the obvious alternative. When the paper introduces equations, reproduce them and define every variable.

### Analogies
For non-obvious concepts, provide an analogy that builds intuition before the formal explanation.

### Results Interpretation
Do not merely restate numbers. Interpret them: What do the ablations reveal about which components matter? Are there surprising results? What does the gap between this method and baselines tell us about the problem structure?

### Emergent Behaviors and Limitations
Weave observations about unexpected behaviors, failure modes, and limitations into the narrative rather than listing them separately. Connect limitations to specific design choices.

### Connections
Cross-reference related work meaningfully: "This is analogous to [X] in [related paper], but differs in [specific way]." Help the reader build a mental map of the field.

## Figure Extraction

If `pdf_extract_figures` is available and a PDF was successfully fetched:

1. Call `pdf_extract_figures` with `pdf_url` set to the PDF URL and `output_dir` set to `<kb_path>/assets/<card-id>/`.
2. Review the returned figure list. Select 2-5 key figures that best illustrate the paper's core method, architecture, or results. Skip decorative figures, logos, and duplicates.
3. Pass the selected figures to `kb_write_card` via the `figures` field, with a short `caption` for each and the `page` number.

If `pdf_extract_figures` is not available, skip this phase silently.

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
<Type 2 pedagogical narrative content — method walkthrough, analogies, results interpretation>

## Key Equations
<Reproduce the 2-3 most important equations with variable definitions and intuitive explanations>

## Connections
<List 3-5 related papers with one-line descriptions of the relationship>
```

If `kb_write_card` is not available, produce both types directly in the chat response.

## Graceful Degradation

- **No KB tools configured**: Output both types in chat; skip card storage.
- **No `paper_get` available**: Rely on `attach_url_files` for the PDF; extract metadata manually from the paper text.
- **PDF download fails**: Synthesize from the abstract and any user-provided context. Clearly note the limitation.
- **User provides only a title**: Search for the paper using available tools before synthesizing. If not found, ask for a URL or arXiv ID.
- **Zotero fulltext unavailable**: Fall back to Path A using the DOI or arXiv ID from Zotero metadata. If no identifier exists, use `paper_search` with the title.
- **No Zotero tools configured**: If the user mentions Zotero but tools aren't available, tell them Zotero integration requires API key configuration and fall back to Path A.
