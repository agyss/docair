# Data: what docair makes and carries

The chart's boxes are the working parts: code, and the places it runs. The
data below is what those parts and you make, and what travels along the lines
between them. A line carries a datum when its payload names it; the page's
**Data** list shows who makes each one and which lines carry it, and *Follow*
lights up its way through the system (SPEC §3.6).

```docair
The data docair makes and carries, as opposed to the code that does the work.

@id      data
@kind    data
@part-of docair
```

```docair
Your question: what you type in the chat panel, in your words. It travels to the server with the graph and the earlier turns.

@id         data.question
@kind       data
@part-of    data
@carried-as questions
```

```docair
The picked files: the project folder you upload, minus dependencies, build output and secrets.

@id         data.files
@kind       data
@part-of    data
@carried-as picked files, upload
```

```docair
The graph: every node, edge, scenario and walkthrough docair found in a project; `docair.graph.json` on disk.

@id         data.graph
@kind       data
@part-of    data
@carried-as Graph
```

```docair
The page: the self-contained HTML that shows the graph, with the viewer and its libraries inside.

@id         data.page
@kind       data
@part-of    data
@carried-as html, generated html, index.html
```

```docair
The digest: the graph shortened for the AI model, without source code.

@id         data.digest
@kind       data
@part-of    data
```

```docair
The model's reply: its answer as JSON, before docair checks it against the graph.

@id         data.reply
@kind       data
@part-of    data
@carried-as reply JSON, answer text, Reply
```

```docair
A walkthrough: the answer as steps through the chart, checked against the graph and recorded as JSON.

@id         data.walkthrough
@kind       data
@part-of    data
@carried-as walkthroughs, Walkthrough
```

```docair
A patch: proposed edits of doc blocks as a unified diff, for the reader to review and apply.

@id         data.patch
@kind       data
@part-of    data
@carried-as diff, PatchAnswer
```
