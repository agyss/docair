# docair-ai

Asks an AI model a question about the graph and gets a **walkthrough** back:
steps in the order things happen, each one naming nodes and edges and saying
what happens and why (SPEC §10). docair binds it to the graph, records it in
`docs/walkthroughs/`, and the viewer replays it.

| Module | Does |
|--------|------|
| `config` | `[ai]` in docair.toml + `DOCAIR_AI_*` + `.env` → `AiConfig` (the key only from the environment or `.env`) |
| `client` | one OpenAI-compatible `POST {endpoint}/chat/completions` call (ureq, rustls) |
| `ask` | digest of the graph, the system prompt, parsing, binding, one repair request |

What leaves the machine is the digest: ids, kinds, names, summaries,
descriptions, file paths and line numbers. Never source code, never the key
in a log or a file.

```docair
The AI endpoint: any API that speaks OpenAI's chat completions, for example Together (https://api.together.xyz/v1) with the model zai-org/GLM-5.3-Flash. It reads the question and the digest and answers with a walkthrough in JSON.

@id      ai-endpoint
@creates data.reply "writes the answer"
@kind    external
@part-of docair
@flow    ask 5 "the model answers with a walkthrough in JSON"
```

```docair
`.env` at the project root: local secrets such as DOCAIR_AI_API_KEY. Gitignored, never scanned or uploaded.

@id      dotenv
@kind    store
@part-of project-files
```
