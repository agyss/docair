# e2e: end-to-end tests

Playwright drives what a user does: generate the docs in the web app, look at
the page docair wrote layer by layer and box by box, hide sections, follow a
walk, clarify with the AI in the page `docair serve` serves (a fake model, so
every run gives the same answers), apply the edits it proposes, and the same
from the CLI. The tests read the page's DOM, its `#` addresses and
`docair.graph.json`; nothing reaches into the page's script.

```bash
npm ci && npx playwright install chromium   # once
npm run build                               # docair, docair-server, the web app
npm test                                    # starts docair-server and the fake model itself
E2E_BASE_URL=http://localhost:8080 npm test # against the kind cluster (skips the fake-model tests)
npm run docs                                # docair build of the repo: the results show on the test nodes
```

Each test has a docair block: `@kind test` and `@covers` what it exercises,
so docair's own page shows the tests next to the parts they cover, with the
result of the last run (`test-results/results.json`, read via `[tests]` in
docair.toml). The **Tests** switch in the page hides them.

| File | What it covers |
|------|----------------|
| `tests/generate.spec.ts` | upload, scan, the page in its frame, secret files left out, New upload |
| `tests/page.spec.ts` | the page docair wrote: each layer by its address, a request through the network as one line, every line with words, at right angles and off the parts, a box opened into its section and Back |
| `tests/levels.spec.ts` | walks (messages, answers, a readable camera, lines pointing ahead), zooming into a box, parts found only in the code, the legend, docair's own lines |
| `tests/layout.spec.ts`, `tests/pick.spec.ts` | a part by its address: where it stays, its home, a link bringing it close, a click selecting it where it is |
| `tests/sections.spec.ts` | hiding and showing sections in the page; `[view] hide` and `show` applied by docair |
| `tests/data.spec.ts` | where a part runs; the Data list; following a datum there and back |
| `tests/tests-layer.spec.ts` | test results on the page, the Tests switch, "Tested by" |
| `tests/ask.spec.ts` | the chat in the page `docair serve` serves, one server per test: walkthroughs, follow-ups, view changes and edits applied to the project, Word the lines, Explain the terms |
| `tests/glossary.spec.ts`, `tests/meanings.spec.ts` | terms explained on hover, Read more in the page, the entry, the Glossary list; words the code defines |
| `tests/apis.spec.ts`, `tests/infra.spec.ts` | API descriptions met with the code; Terraform, Helm and manifests |
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
