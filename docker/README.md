# docair in Docker

Two images: `docair-server` (the server binary on a distroless base) and
`docair-web` (the Angular app, served by nginx, which passes `/api` on to the
server).

```sh
docker build -f docker/server.Dockerfile -t docair-server .
docker build -f docker/web.Dockerfile -t docair-web .
docker network create docair
docker run -d --network docair --name docair-server docair-server
docker run -d --network docair -p 8080:8080 docair-web
# open http://localhost:8080, drop a project folder, click "Generate docair page"
```

`docair-web` finds the server at `DOCAIR_SERVER` (host:port, default
`docair-server:8080`). The server reads its settings from the environment:

| Variable | What | Default |
|----------|------|---------|
| `DOCAIR_MAX_UPLOAD_MB` | the biggest upload it takes | 25 |
| `DOCAIR_MAX_FILES` | the most files in one upload | 5000 |
| `DOCAIR_AI_ENDPOINT`, `DOCAIR_AI_MODEL`, `DOCAIR_AI_API_KEY` | the AI for asking about a page; unset, asking is off | unset |

`docair-server` alone also serves the web app when `DOCAIR_STATIC_DIR` names
its built files (`web/dist/docair-web/browser`), as the e2e tests run it.

The parts that have no code of their own are declared here:

```docair
The machine that runs docair: your laptop, or the devcontainer. The docair
CLI, the editor and the browser all run here.

@id      workstation
@kind    host
@part-of docair
```

```docair
Docker Engine: builds the two images and runs their containers.

@id      docker
@kind    host
@part-of workstation
```

```docair
Docker's local image store: docair-server and docair-web, as built from
`docker/`.

@id      images
@kind    store
@part-of docker
```

```docair
The scratch folder of the server, /tmp in its container: each upload is
written here, scanned and kept as a project (at most 8, dropped after an hour
unused).

@id      upload-dir
@kind    store
@part-of container.docair-server
```
