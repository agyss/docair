# docair-viewer

Renders a docair graph as **one self-contained HTML file**: the graph JSON,
the rendered Markdown, the viewer script and its two libraries are all
inlined, so the page works offline, from `file://`, as a CI artifact or as a
mail attachment.

| Part | What it does |
|------|--------------|
| [Cytoscape.js](https://js.cytoscape.org) | drawing, compound nodes (`@part-of` frames), styles per kind, pan/zoom, events, animation |
| [ELK](https://eclipse.dev/elk/) (elkjs) | layered layout of what is inside each box, one box at a time |
| `assets/viewer.js` | top-down placement (sizes the boxes), the line router, tree, details, collapse/expand, search, diagnostics, flow player |

## Vendored libraries

Pinned and checked in, so builds need no network. Update them on purpose,
not by accident.

| File | Version | License | sha256 |
|------|---------|---------|--------|
| `vendor/cytoscape.min.js` | cytoscape 3.34.4 | MIT, `vendor/LICENSE-cytoscape.txt` | `081591ab735b2b6a3b31a3c60f8f3a3e2fee8c92939e0c7820ad1208571b6cd8` |
| `vendor/elk.bundled.js` | elkjs 0.12.1 | EPL-2.0, `vendor/LICENSE-elkjs.md` | `b1032b7cfd2a57c54c6f0e8369d7665e19a579954f7cd20a6dae28f1912ea0d7` |

elkjs is EPL-2.0: the generated page carries a notice that names the license
and the source repository.

## The page and its readers

```docair
`docair.html`: the self-contained viewer page.

@id      html-file
@kind    store
@part-of workstation
@flow    scan 13 "docair.html is on disk"
```

```docair
Your web browser: it opens the docair web app on localhost:8080, and it shows the generated pages.

@id      browser
@kind    external
@part-of docair
@reads   html-file "opens the viewer page"
@calls   host-port (HTTP GET /) -> index.html, app bundle
@flow    scan 14 "someone opens the page"
@flow    open 1 "you open http://localhost:8080"
```
