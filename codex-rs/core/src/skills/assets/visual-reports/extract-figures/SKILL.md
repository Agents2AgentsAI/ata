---
name: extract-figures
description: Extract figures from research paper PDFs and store them in KB cards. Use when a user wants to extract high-quality figures from a paper PDF into their knowledge base.
metadata:
  short-description: Extract figures from paper PDFs
---

# Extract Figures

Extract high-quality figures from research paper PDFs and store them in KB cards.

## Prerequisites

- `pdf_extract_figures` tool must be available.
- A PDF must have been fetched (via `attach_url_files` or a local path).
- Call `kb_status` first to get `kb_path`.

## Extraction Workflow

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
   - When in doubt between a results chart and an architecture diagram, always pick the architecture diagram. The narrative can describe numerical results in text, but cannot substitute for a visual system overview.

4. **Move only the selected figures** into the KB assets directory: `mkdir -p <kb_path>/assets/<card-id>/ && cp <selected-figure-paths> <kb_path>/assets/<card-id>/`. Then delete the temp directory (`rm -rf /tmp/pdf-figures-<card-id>/`).

5. Pass the selected figures to `kb_write_card` via the `figures` field, using their new paths under `assets/<card-id>/`, with a short `caption` for each and the `page` number.

## Graceful Degradation

- **No `pdf_extract_figures` available**: Report that figure extraction requires the `pdf_extract_figures` tool.
- **PDF not available**: Cannot extract figures without a PDF source. Ask the user for a PDF URL or path.
- **No figures extracted**: Some papers have no extractable figures (text-heavy, or figures are embedded in unusual formats). Report this and move on.
- **No KB tools configured**: Output figure paths and captions directly in chat instead of storing in KB.
