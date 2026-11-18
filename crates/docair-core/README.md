# docair-core

The language-agnostic pipeline. It knows comment syntaxes and the tag grammar
of [SPEC.md](../../docs/SPEC.md), nothing else. Languages plug in through the
`Adapter` trait.

| Stage | Module | Output |
|-------|--------|--------|
| 1 discover | `discover` | `SourceFile[]` |
| 2 extract | `extract`, `markdown` | `RawBlock[]`, `MdDoc[]` |
| 3 enrich | `adapter` | `RawBlock[]` with hints and inferred tags |
| 4 build | `build` | `Node[]` |
| 5 resolve | `resolve` | `Edge[]`, `Flow[]` |
| 5 resolve | `walkthrough` | recorded walkthroughs, bound to the graph |
| 6 check | `check` | `Diagnostic[]` |
| 7 emit | `emit` | `docair.graph.json` |

The stores the pipeline reads and writes have no code of their own, so they
are declared here:

```docair
The scanned project on your disk: source files, Markdown and `docair.toml`.

@id      project-files
@kind    store
@part-of workstation
```

```docair
`docair.graph.json`: the versioned graph schema that the viewer and other tools read.

@id      graph-json
@kind    store
@part-of workstation
```

```docair
The recorded walkthroughs, `docs/walkthroughs/*.json`: questions answered step by step through the graph, kept in the repository and replayed by every page.

@id      walkthrough-files
@kind    store
@part-of project-files
```

```docair
Test reports: the Playwright JSON reports named in `[tests] results` (e2e/test-results/results.json). Every scan puts their results on the test nodes.

@id      test-reports
@kind    store
@part-of project-files
```

```docair
The glossaries: `GLOSSARY.md` files (and those listed in `glossary` in docair.toml), one `##` heading per term. Every scan reads them; the page explains their terms where the docs use them.

@id      glossary-files
@kind    store
@part-of project-files
```

```docair
The decisions about words: `docs/meanings.toml` (or `meanings` in docair.toml). Which part's docs define a word, which words are ordinary, and which glossary entries were compared with the code (docs/MEANINGS.md). `docair glossary` writes it; every scan reads it.

@id      meanings-file
@kind    store
@part-of project-files
```
