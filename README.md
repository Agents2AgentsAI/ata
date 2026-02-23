<p align="center"><code>npm i -g @a2a-ai/ata</code><br />or <code>curl -fsSL https://agents2agents.ai/ata/install.sh | sh</code></p>
<p align="center"><strong>Ata CLI</strong> is an AI assistant from Agents2Agents AI that runs locally on your computer.<br />It is built on top of OpenAI Codex CLI. Not affiliated with OpenAI.
<p align="center">
  <img src="https://github.com/Agents2AgentsAI/ata/blob/main/.github/cli-splash.png" alt="Ata CLI splash" width="80%" />

---

## Quickstart

### Installing and running Ata CLI

Install globally with your preferred package manager:

Install using npm

```shell
npm install -g @a2a-ai/ata
```

Install using homebrew (macOS)

```
brew install --cask ata
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

You can use `ata` with ChatGPT plan or OpenAI, Anthropic, or Gemini API key. All your API keys are stored securely on your local machine and never sent anywhere.

## Docs

- **Coming soon** - consult [Ata docs](https://github.com/Agents2AgentsAI/ata/tree/main/docs) for now
- [**Installing & building**](./docs/install.md)
- [**Setting up Zotero**](./docs/zotero-setup.md)

This repository is licensed under the [Apache-2.0 License](LICENSE).
