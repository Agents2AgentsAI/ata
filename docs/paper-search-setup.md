# Setting Up Paper Search with Ata

## Overview

Ata can search for academic papers across three sources simultaneously:

- **[Semantic Scholar](https://www.semanticscholar.org/)** — AI-powered academic search engine
- **[arXiv](https://arxiv.org/)** — open-access preprint repository
- **[OpenAlex](https://openalex.org/)** — open catalog of scholarly works

Paper search works out of the box with no API keys required. Optional credentials can improve rate limits.

## Tools Available To Agents

| Tool                    | Description                                                      |
| ----------------------- | ---------------------------------------------------------------- |
| `paper_search`          | Search papers across all three sources                           |
| `paper_get`             | Get detailed paper info by DOI, arXiv ID, or Semantic Scholar ID |
| `paper_citations`       | Get papers that cite a given paper                               |
| `paper_references`      | Get papers referenced by a given paper                           |
| `paper_recommendations` | Get recommendations based on example papers                      |

## Optional Configuration

All environment variables below are optional. Paper search works without any of them.

| Variable                   | Description                                                                                                                                                                                        |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SEMANTIC_SCHOLAR_API_KEY` | Increases rate limits on non-search Semantic Scholar endpoints. Search is capped at 1 RPS regardless. Get one at [Semantic Scholar API](https://www.semanticscholar.org/product/api#api-key-form). |
| `OPENALEX_EMAIL`           | Courtesy email for OpenAlex requests. Provides access to the polite pool with better rate limits.                                                                                                  |

Set them in your shell profile:

```shell
export SEMANTIC_SCHOLAR_API_KEY="your-key"
export OPENALEX_EMAIL="you@example.com"
```

## Usage with Ata

Once Ata is running, you can ask it to search for papers naturally:

- "Find recent papers on transformer architectures"
- "Get citations for arXiv:2301.00001"
- "What papers are similar to this DOI: 10.1234/example?"
