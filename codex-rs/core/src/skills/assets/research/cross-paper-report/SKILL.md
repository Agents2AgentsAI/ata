---
name: cross-paper-report
description: Generate integrated cross-paper explanations from KB cards. Use when a user asks to explain, compare, synthesize, understand, or deep-dive into one or more knowledge base cards, or asks how cards relate or work together.
---

# Cross-Paper Report

> **When to use this skill vs. alternatives:**
> This skill produces exhaustive deep reports (800+ words per paper, full comparative synthesis). For quick orientations, use `$research-briefing` instead — it gives a 2-4 page overview with 3-5 sentences per paper. For conversation-derived reports that capture what you specifically discussed, use `$conversation-report` instead. Use this skill when the user wants a complete technical reference document. For LaTeX PDF output with TikZ diagrams, use `$latex-report` after generating the narrative.

You MUST produce a **deep narrative explanation** — no exceptions, no shortcuts. Focal cards: 800–2500 words; supporting cards: 400–600 words (see Tiered Depth Strategy).

A short summary is NEVER acceptable. This task is NOT complete until the deep narrative exists.

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
- **Never reference the KB in explanations.** Do not say "as summarized in your KB" or "your KB card says." The KB is infrastructure — present content as if you understand the papers directly.
- **No figure-reference sections.** The reading view is text-only — images and figures cannot be displayed. Never include sections like "Figure Pointers", "How to view figures", or "Key Figures" that tell the user to look at specific figures by number. Instead, describe what each important figure shows inline in the narrative (e.g., "The architecture diagram in the paper shows three stages connected by…"). This applies to `present_reading_view` content and chat explanations alike.
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

Determine `<kb_path>` per the `$kb` skill (default `~/.ata/knowledge-base` unless configured otherwise).

## Scale Detection

After reading all requested cards, check the count:

- **Standard mode** (≤ 12 cards): Follow Phases 1–2 as written below. Every card gets a full deep walkthrough in one pass.
- **Large-set mode** (> 12 cards): Jump to the **Large-Set Synthesis** section at the end of this document. Do NOT attempt standard Phases 1–2 for large sets — producing 800+ words per card for 20+ papers in a single agent pass will collapse into shallow summaries that violate the depth contract. The large-set workflow uses clustering and parallel subagents to preserve per-card depth at scale.

## Phase 0: Related Card Discovery

After reading the requested cards but before writing explanations, check whether the KB contains other cards that are topically relevant and would strengthen the report.

1. List all cards per `$kb` to see all available cards.
2. For each requested card, scan its `tags`, `connections`, and the topics mentioned in its body. Identify other KB cards that share tags, are cited in connections, or address closely related methods.
3. If relevant unrequested cards exist, present them to the user:
   > "Your KB also has cards for [list] which are related to this topic. Want me to include any of them? Adding them would strengthen the comparison by covering [specific dimension, e.g., 'alternative action representations' or 'data augmentation methods']."
4. If the user approves additions, include them in the card set for Phases 1–2. Re-check Scale Detection with the updated count.

This step prevents narrow reports that miss important context available in the KB. Skip only if the user explicitly says "only these cards."

**Subagent context:** Skip Phase 0 entirely when running as a cluster subagent in the Large-Set Synthesis workflow. Cluster subagents work only on their assigned cards — discovering additional cards would cause cross-contamination between clusters. Phase 0 runs only in the main agent before clustering.

## Phase 1: Deep Technical Explanation of Each Card

Read all requested cards per `$kb` (or list + read all cards when the user says "all" or refers to a broad set).

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

**Research context awareness:** Before writing the comparison, read `<kb_path>/research-context.md` (if it exists). If the user has documented priorities (e.g., "inference latency matters most"), frame comparative dimensions around those priorities — lead with the dimensions the user cares about, de-emphasize dimensions they've marked as unimportant.

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

## Depth Gate (Mandatory Before Completion)

Before finalizing, verify depth mechanically:

1. **Per-card word count.** For each card's section in the explanation, estimate word count. Focal cards must be at least 800 words; supporting cards must be at least 400 words. If ANY card section is below its tier's floor, STOP and expand it. Re-read the relevant KB card per `$kb` and deepen the shallow section.
2. **Equation intuitions.** Check that every equation has a full-paragraph intuition (3–5 sentences explaining why it works and what happens at boundary conditions), not a one-liner like "continuous motion is snapped to a symbol."
3. **Stage walkthrough depth.** Check that every stage in a stage-by-stage walkthrough has 2–3 full paragraphs — not a single sentence. A one-sentence stage description violates the depth contract.
4. **Comparison specificity.** Check that the cross-card comparison has at least one full paragraph per dimension with concrete numbers and implementation details traced from individual cards — not generic claims like "X is bigger than Y."

If depth is insufficient at any checkpoint, expand before finalizing.

## Presentation (Main Agent Only)

**This section applies to the main agent only.** Cluster subagents in the Large-Set workflow return their content as text to the main agent — they never call `present_reading_view` because the user is interacting with the main agent.

**Phase 1 (Outline):** IMMEDIATELY call `present_reading_view` with `document_id` set to a unique slug, `title` to the report title, and `content` containing ONLY the `## ` section headings with empty bodies. Example content: `"## Introduction\n\n## Core Method\n\n## Experiments\n\n## Discussion"`. This opens the reading view instantly with "Generating..." placeholders.

**Phase 2 (Fill):** The tool result will tell you to fill section 0. Immediately call `update_document_section(document_id, section_index=0, content="...")` with the FULL content for that section — do not output any text, just make the tool call. Each tool result tells you the next section to fill. Continue calling `update_document_section` for each subsequent section until all are filled.

**Markdown formatting:** Always put a blank line before numbered list items (`1.`, `2.`, etc.) and before bullet list items (`-`, `*`). Without a blank line, the markdown parser treats `2.`, `3.`, etc. as plain text instead of list items, so they lose their formatting. This also applies to content after paragraphs, blockquotes, and code blocks.

When the user asks follow-up questions about a specific section, use the most efficient update tool:
- `append_to_section` — to add new information at the end of a section (most common for follow-up questions)
- `patch_document_section` — to change specific text within a section (for corrections or targeted edits)
- `update_document_section` — to fully rewrite a section (only when the entire section needs to change)

Write follow-up answers as straight content — no editorial labels like "(clearer explanation)" or "(expanded)" in headings or topic lines.

## Post-Report Housekeeping

After the report is complete (both reading view + PDF delivered), do these:

**1. Journal entry** — Append to `<kb_path>/research-journal.md`. Prepend (newest first):

```markdown
## [Date] — Cross-Paper Report: [Topic/Title]

### Explored
- Compared [N] papers: [list paper titles briefly]
- Focus dimensions: [e.g., action tokenization, inference latency, training strategy]

### Key Findings
- [1-2 bullet points: the most important comparative insights]

### Cards Touched
- [card-ids] (read)

---
```

**2. Research context detection** — During the report interaction and follow-ups, watch for preference signals. If the user focuses on specific comparison dimensions ("I really care about the latency comparison"), asks to skip sections, or responds well to particular framings, offer briefly to note it in `research-context.md`. Never block the interaction on this.

## Completion Checklist

Before reporting done, verify ALL of these:
- [ ] All relevant cards included (none dropped for depth budget — use tiered depth if needed)
- [ ] Deep narrative prose in chat (focal cards: 800+ words; supporting cards: 400+ words)
- [ ] Depth Gate passed (word counts, equation intuitions, stage walkthrough depth, comparison specificity)
- [ ] Journal entry appended to `research-journal.md`

## Anti-Patterns (Things You Must NEVER Do)

Every item below is a failure. If you catch yourself doing any of these, stop and fix it before proceeding.

- NEVER compress a stage walkthrough into a single sentence. A stage like "Stage 2: The encoder pseudo-labels videos; a VLM predicts those tokens" is a failure — every stage must be 2–3 full paragraphs.
- NEVER write a one-line equation intuition. "Intuition: continuous motion is snapped to a symbol" is a failure — every equation intuition must be a full paragraph (3–5 sentences).
- NEVER write shallow bullet-point summaries like "Core idea: uses VQ-VAE for latent actions" without explaining how or why the mechanism works.
- NEVER use a comparison table as the sole comparison mechanism. Tables flatten nuance — they can only appear alongside full-paragraph prose comparisons.
- NEVER write one-liner role descriptions like "Role in a stack: pretraining backbone" — that has zero technical substance.
- NEVER parrot card titles and tags as if they were an explanation. Restating "LAPA: Latent Action Pretraining" without walking through the mechanism is not an explanation.
- NEVER cover only the explicitly requested cards when the KB has closely related cards. Failing to check for related cards (Phase 0) produces a report with blind spots.
- NEVER drop cards to save depth budget. Covering 3 of 10 relevant papers deeply is worse than covering all 10 at mixed depth. Use Tiered Depth or Large-Set Synthesis instead of cutting cards.

---

## Large-Set Synthesis (> 12 cards)

When the card set exceeds 12, a single-pass deep walkthrough will collapse into surface-level summaries — the agent runs out of output capacity before covering every card at 800+ words. This section defines the mandatory hierarchical synthesis workflow that preserves per-card depth at scale.

**Do NOT skip this workflow for large sets.** The standard Phases 1–2 are designed for 2–12 cards. For 13+ cards, always use this workflow instead.

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

Launch **one subagent per cluster**, all in parallel. Each subagent runs the full standard cross-paper-report workflow (Phases 1–2) on its cluster's cards only:

- Full 800–2500 word per-card deep walkthroughs with all cards treated as focal (Phase 1)
- Within-cluster comparative synthesis (Phase 2)
- Depth Gate verification before proceeding

**Subagent prompt template:**

> Invoke the `cross-paper-report` skill for cards `[card-id-1]`, `[card-id-2]`, ..., `[card-id-N]`. The KB path is `[kb_path]`. This is a cluster sub-report titled "[Cluster Name]".
>
> Follow the skill's standard workflow (Phases 1–2) with these overrides:
> - **Skip Phase 0** (Related Card Discovery) — you work only on the assigned cards. Do NOT list cards to discover additional ones.
> - **Skip the Presentation section** — do NOT call `present_reading_view`. The user is on the main agent, not on you. Return your content as text to the main agent.
> - **All cards are focal** — no tiered depth. Every card gets the full 800–2500 word treatment.
> - **No user confirmation** — do not present tier assignments or card additions for approval. Execute autonomously.
> - **Shared terminology glossary:** [paste the glossary from Step 1 here so all subagents use consistent terms]
> - Produce deep per-card walkthroughs (800+ words each) and within-cluster comparative synthesis.
> - Return to the main agent:
>   1. The full content (per-card walkthroughs + within-cluster synthesis)
>   2. A 3–5 sentence cluster summary
>   3. A **cross-cluster interface** section containing: (a) key technical terms defined and used in this cluster with definitions, (b) 2–3 key equations with paper attribution, (c) specific numbers/results that the meta-synthesis may reference for cross-cluster comparison, (d) bridge points — concepts in this cluster that connect to papers in other clusters

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

### Large-Set Completion Checklist

Before reporting done, verify ALL of these:
- [ ] Cards clustered into 3–6 groups of 4–8 cards each
- [ ] Shared terminology glossary defined and included in every subagent prompt
- [ ] Every cluster sub-report meets the depth floor (800+ words per card, all cards focal)
- [ ] Every cluster subagent returned a cross-cluster interface section
- [ ] Failed subagents (if any) are noted with missing card lists and retry offered
- [ ] Meta-synthesis contains all required sections (evolution tracing, design space mapping, failure mode lineage, equations, How the Papers Relate)
- [ ] Meta-synthesis uses concrete data from cross-cluster interface sections, not generic claims

### Large-Set Anti-Patterns

These are the specific failure modes that large-set synthesis must avoid:

- **One-sentence-per-paper contribution maps** — listing card IDs with a single sentence each. Every card must have its full 800+ word walkthrough inside its cluster sub-report.
- **Thematic overview without per-card depth** — writing a coherent narrative about the "field trajectory" while skipping the per-card technical walkthroughs. The meta-synthesis sits ON TOP of per-card depth, not instead of it.
- **Equations without cross-cluster tracing** — including equations but not explaining how different papers instantiate the same equation differently.
- **Generic comparison claims** — "X is better than Y" without concrete numbers, architectural specifics, or implementation details from the actual papers.
