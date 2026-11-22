# Contributing to docair

Thank you for helping. docair is published under the
[Elastic License 2.0](LICENSE), and its owner can only take a contribution
under the [Contributor License Agreement](CLA.md): you transfer your
contribution to him, so he can keep licensing docair as he chooses. Agree to
it in the description of your first pull request (CLA.md §10).

## Before you send a pull request

```bash
cargo test --workspace                      # unit tests and the golden graphs
cargo clippy --workspace --all-targets      # no warnings
cargo fmt --all --check
cargo run -p docair-cli -- check .          # docair on itself: 0 errors
cd e2e && npm run build && npm test         # the end-to-end tests (Playwright)
```

* **docair documents itself.** New code gets docair doc blocks (`/*** … ***/`
  with `@id`, `@part-of`, `@calls`, …; docs/SPEC.md), and `docair check .`
  stays at 0 errors.
* **Golden files** (`crates/docair-adapters/tests/golden/`,
  `crates/docair-viewer/tests/golden/`): a change that moves them says why.
  Rewrite them with `DOCAIR_BLESS=1 cargo test -p <crate>` and look at the
  diff.
* **Docs are plain English**: short sentences, tables, examples. A larger
  change starts as a design doc in `docs/`; docs/SPEC.md says what is built.
* **Dependencies**: only versions at least two weeks old, the choice and why
  in docs/LIBRARIES.md.

The design is in [docs/SPEC.md](docs/SPEC.md) and
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
