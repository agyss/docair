# e2e: end-to-end tests

Playwright drives what a user does: generate the docs in the web app, look at
them layer by layer, hide sections, follow a walkthrough, clarify with the
AI (a fake model, so every run gives the same answers), apply the edits it
proposes, and the same from the CLI.

```bash
npm ci && npx playwright install chromium   # once
npm run build                               # docair, docair-server, the web app
npm test                                    # starts docair-server and the fake model itself
E2E_BASE_URL=http://localhost:8080 npm test # against the kind cluster (skips the fake-model tests)
npm run docs                                # docair scan of the repo: the results show on the test nodes
```

Each test has a docair block: `@kind test` and `@covers` what it exercises,
so docair's own page shows the tests next to the parts they cover, with the
result of the last run (`test-results/results.json`, read via `[tests]` in
docair.toml). The **Tests** switch in the page hides them.

| File | What it covers |
|------|----------------|
| `tests/generate.spec.ts` | upload, scan, the page, secret files left out, New upload |
| `tests/levels.spec.ts` | layers (each draws its own concern; a request through the network is one line between the apps), zooming into a box, every line has words, walks at the layer that is about them with Open and Back, at a readable size |
| `tests/sections.spec.ts` | hiding and showing sections; `[view] hide` |
| `tests/tests-layer.spec.ts` | test results on the page, the Tests switch, "Tested by" |
| `tests/ask.spec.ts` | the conversation: walkthroughs, follow-ups, view changes, edits applied and regenerated, Word the lines, Explain the terms |
| `tests/glossary.spec.ts` | terms explained on hover, Read more in the page, the entry, the Glossary list |
| `tests/cli.spec.ts` | `docair ask`, `--follow-up`, `docair chat`, `docair apply`, `docair word`, `docair glossary`, `--apply` into docair.toml |
| `fixtures/shop/` | the project the tests document: Rust, TypeScript, Terraform, a scenario, tests with results |

```docair
The end-to-end tests: Playwright drives the web app, the generated page and the CLI, as a user would.

@id      e2e
@kind    test
@part-of docair
```

```docair
A fake AI endpoint for the tests: it speaks OpenAI's chat completions and answers by keywords, so every run gets the same replies.

@id      e2e.fake-ai
@kind    external
@part-of e2e
```
