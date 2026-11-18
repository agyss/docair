# docair-web

The frontend of the docair web app (Angular 22). Pick or drop a project
folder, click *Generate*, and the page docair generates for it fills the
window. Each step says how far it is: the upload in bytes, docair reading the
files in seconds, the page coming back in bytes, and the page drawing itself
until it is ready (docs/SPEC.md §1.3). At the right, **Ask docair** is a conversation with the configured
model (docs/SPEC.md §10): replies play walkthroughs on the page, hide or show
sections (with Undo), and propose edits of the doc blocks, which can be
reviewed as a diff, applied to the uploaded files (the page is generated
again) or downloaded as `docair.patch`.

| File | Does |
|------|------|
| `upload-panel.ts` | picks or takes dropped folders; leaves out dependencies, build output and secrets |
| `progress-line.ts` | where a generation is: its step, a bar, bytes or seconds |
| `result-view.ts` | the page in a sandboxed frame, messages to and from it, edits applied and regenerated |
| `chat-panel.ts` | the conversation: questions, replies, Play, Undo, Review diff, Apply & regenerate |
| `conversation.service.ts` | the turns, kept across regenerated pages until a new upload |
| `ask.service.ts`, `scan.service.ts` | `/api/ask`, `/api/patch`, `/api/ai`, `/api/scan` |

```bash
npm ci
npm run build                 # dist/docair-web/browser, served by nginx in the image
npx ng serve                  # dev server on :4200, /api proxied to localhost:8081
```

Run the backend next to it with
`DOCAIR_BIND=127.0.0.1:8081 cargo run -p docair-server`, or let the server
serve the built app too:
`DOCAIR_STATIC_DIR=web/dist/docair-web/browser cargo run -p docair-server`.

```docair
docair-web: the Angular app. nginx in the docair-web container serves it; it
runs in the browser and talks to docair-server through /api.

@id      docair-web
@kind    service
@part-of container.docair-web
@runs-in browser
```
