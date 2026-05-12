<p align="center"><code>npm i -g @a2a-ai/ata</code><br />or <code>curl -fsSL https://agents2agents.ai/ata/install.sh | sh</code></p>
<p align="center"><strong>Ata CLI</strong> is an AI assistant from Agents2Agents AI that runs locally on your computer.<br />It is built on top of OpenAI Codex CLI. Not affiliated with OpenAI.
<p align="center">
  <img src="https://github.com/Agents2AgentsAI/ata/blob/main/.github/cli-splash.png" alt="Ata CLI splash" width="80%" />

---

We are building **Ata** to be the AI assistant that can solve software engineering _and_ research problems end-to-end - not just another code-assist tool. On top of the vanilla Codex CLI, Ata adds:

- **Multiple AI providers**: OpenAI, Anthropic, and Gemini. We also provide on-premise self-hosted solutions; contact us at accounts@agents2agents.ai for details.
- **Research tools**: Paper Search (Semantic Scholar, arXiv, OpenAlex), Patent Search (EPO), and Zotero integration
- **LSP and Tree-sitter support**: for deeper semantic understanding of your codebase
- **Voice support**: powered by ElevenLabs
- **Research View**: a dedicated reading experience for papers and patents

And many more capabilities coming soon.

---

## Quickstart

### Installing and running Ata CLI

Install globally with your preferred package manager:

Install using npm

```shell
npm install -g @a2a-ai/ata
```

Install using curl (macOS/Linux)

```shell
curl -fsSL https://agents2agents.ai/ata/install.sh | sh
```

Then simply run `ata` to get started.

<details>
<summary>You can also go to the <a href="https://github.com/Agents2AgentsAI/ata/releases/latest">latest GitHub Release</a> and download the appropriate binary for your platform.</summary>

Each GitHub Release contains many executables, but in practice, you likely want one of these:

- macOS
  - Apple Silicon/arm64: `ata-aarch64-apple-darwin.tar.gz`
  - x86_64 (older Mac hardware): `ata-x86_64-apple-darwin.tar.gz`
- Linux
  - x86_64: `ata-x86_64-unknown-linux-musl.tar.gz`

Each archive contains a single entry with the platform baked into the name (e.g., `ata-x86_64-unknown-linux-musl`), so you likely want to rename it to `ata` after extracting it.

</details>

### Using Ata with different providers

Run `codex` and select **Sign in with ChatGPT**. We recommend signing into your ChatGPT account to use Codex as part of your Plus, Pro, Business, Edu, or Enterprise plan. [Learn more about what's included in your ChatGPT plan](https://help.openai.com/en/articles/11369540-codex-in-chatgpt).

You can also use Codex with an API key, but this requires [additional setup](https://developers.openai.com/codex/auth#sign-in-with-an-api-key).

## Docs

- [**Installing & building**](./docs/install.md)
- [**Setting up LSP & Tree-sitter**](./docs/lsp-treesitter-setup.md)
- [**Setting up Paper Search**](./docs/paper-search-setup.md)
- [**Setting up Patent Search**](./docs/patent-search-setup.md)
- [**Setting up Zotero**](./docs/zotero-setup.md)
- [**Adding a ChatGPT subscription to the shared pool**](./codex-rs/supabase/scripts/README.md)

This repository is licensed under the [Apache-2.0 License](LICENSE).
