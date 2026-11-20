//! docair-core: the language-agnostic model and pipeline (see `docs/SPEC.md`).

use anyhow::Result;
use std::path::Path;

/***
 * Stage 3: the `Adapter` trait; adapters read the code and report facts.
 ***/
pub mod adapter;
/***
 * Stage 4: blocks become nodes with ids, kinds, parents and docs.
 ***/
pub mod build;
/***
 * Stage 6: graph-wide rules and the severity policy.
 ***/
pub mod check;
/***
 * `docair.toml`: system id, include/exclude globs, strictness.
 ***/
pub mod config;
/***
 * What two commits change in the system's flows: the changed parts and the slices through them, compared (docs/CHANGES.md).
 ***/
pub mod changes;
/***
 * The steps of a change: a changed flow before and after as one picture, its marks, and its changes in steps with stable ids, for the page's walk and the model's words.
 ***/
pub mod change_steps;
/***
 * The review: the pieces of a whole change across every flow, the intent beside them, and a change's own picture.
 ***/
pub mod review;
/***
 * How much of what authors wrote the code confirms: `docair check --coverage`.
 ***/
pub mod coverage;
/***
 * What two graphs of a project differ in: parts, lines, and the parts that only moved (docs/CHANGES.md §3).
 ***/
pub mod diff;
/***
 * What API descriptions declare, meeting the code: endpoints, merged into the handlers that serve their routes.
 ***/
pub mod declare;
/***
 * Stage 1: walk the project, honour ignore files and globs.
 ***/
pub mod discover;
/***
 * Stage 7: the `docair.graph.json` file.
 ***/
pub mod emit;
/***
 * Edits of doc blocks: where they go, the new file texts and a unified diff.
 ***/
pub mod edit;
/***
 * Environment variables: the reads the code makes meet what sets them.
 ***/
pub mod env;
/***
 * Stage 2: doc blocks found by comment syntax, split into description and tags.
 ***/
pub mod extract;
/***
 * What adapters learn from the code: symbols, the routes they serve, their relations.
 ***/
pub mod facts;
pub mod ids;
/***
 * The language registry: extension to comment family.
 ***/
pub mod lang;
/***
 * What each layer draws: the parts it is about, their boxes, context, closed boxes, and lines through left-out parts.
 ***/
pub mod layer_view;
/***
 * The glossary: what the words of the docs mean, and the docs it links to.
 ***/
pub mod glossary;
/***
 * Stage 2 for Markdown: anchored sections, fenced docair blocks, sibling docs.
 ***/
pub mod markdown;
/***
 * What the words of the docs mean: from the code, the glossary and doc headings, the code first.
 ***/
pub mod meanings;
/***
 * Doc blocks meet the facts: each block is attached to the symbol it documents.
 ***/
pub mod merge;
pub mod model;
/***
 * Stage 5: refs become edges, flows get their paths.
 ***/
pub mod resolve;
/***
 * Requests the code sends meet the routes that serve them: lines, W116, W117, W118.
 ***/
pub mod request;
/***
 * Routes and requested paths: segments, parameters in every spelling, prefixes, and how well a request fits a route.
 ***/
pub mod route;
/***
 * Function signatures as wording for the edges that call them.
 ***/
pub mod signature;
/***
 * Slices: one part of the system, from a seed (a datum, a part, a route, a walk) to its ends.
 *
 * @doc ../../../docs/SLICES.md
 ***/
pub mod slice;
/***
 * Test results from Playwright reports, put on the test nodes.
 ***/
pub mod test_results;
/***
 * Tidy: the hand-written tags that only repeat what the code says, and the edits that remove them (docs/ADAPTERS.md §6.1).
 ***/
pub mod tidy;
/***
 * Recorded walkthroughs: read, bound to the graph, written.
 ***/
pub mod walkthrough;

use adapter::Adapter;
use lang::Family;
use model::{Graph, SCHEMA, View};

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Unresolved refs are warnings instead of errors.
    pub lenient: bool,
    /// The project comes from someone else (an upload to the server): docair
    /// runs no programs on it, so `[code] run_indexers` is ignored.
    pub untrusted: bool,
}

/***
 * Run stages 1 to 6 on a project and return the graph with its diagnostics.
 *
 * @flow  scan 2 "the pipeline starts"
 * @flow  check 2 "stages 1 to 6 run"
 * @flow  ask 2 "stages 1 to 6 run"
 * @flow  upload 14 "docair builds the uploaded folder"
 * @calls docair-core.config.load "reads docair.toml or the defaults"
 * @calls docair-core.adapter.collect "lets the adapters read the code"
 * @calls docair-core.env.link "gives reads of environment variables lines from what sets them"
 * @calls docair-core.merge.merge "attaches the blocks to the code they document"
 * @calls docair-core.request.link "gives requests lines to what serves them"
 * @calls docair-core.resolve.flow_paths "orders the scenario steps and links them, request lines included"
 * @calls docair-core.walkthrough.load "reads the recorded walkthroughs"
 * @calls docair-core.test_results.attach "puts the results on the test nodes"
 * @calls docair-core.glossary.parse "reads the glossary"
 * @calls docair-core.glossary.library "embeds the docs the glossary links to"
 * @calls docair-core.meanings.load "reads the decisions about words"
 * @calls docair-core.meanings.settle "settles what the words mean, the code first"
 * @creates data.graph "builds the graph of the project"
 * @doc   ../../../docs/ARCHITECTURE.md
 ***/
pub fn scan(root: &Path, adapters: &[&dyn Adapter], opts: &Options) -> Result<Graph> {
    let mut config = config::load(root)?;
    if opts.untrusted
        && let Some(code) = config.code.as_mut()
    {
        code.run_indexers = false;
    }
    let system = config.system_id(root);
    let files = discover::discover(root, &config)?;
    let mut diags = Vec::new();

    let (md_files, code_files): (Vec<_>, Vec<_>) = files
        .iter()
        .cloned()
        .partition(|f| f.lang.family == Family::Markdown);
    // Glossaries define words, not nodes (SPEC §13).
    let (glossary_files, md_files): (Vec<_>, Vec<_>) = md_files
        .into_iter()
        .partition(|f| glossary::is_glossary(&f.rel, &config));
    let terms: Vec<_> = glossary_files
        .iter()
        .flat_map(|f| glossary::parse(&f.rel, &f.text))
        .collect();
    let docs: Vec<_> = md_files
        .iter()
        .map(|f| markdown::read_markdown(f, &mut diags))
        .collect();
    let blocks: Vec<_> = code_files
        .iter()
        .map(|f| extract::extract(f, &mut diags))
        .collect();
    let project = adapter::Project {
        system: &system,
        config: &config,
        files: &code_files,
        blocks: &blocks,
    };
    let mut facts = adapter::collect(&project, adapters);
    env::link(&mut facts);
    let blocks = merge::merge(&code_files, blocks, &facts, &mut diags);
    // Declared blocks first: a discovered node with the same id yields to them.
    let mut blocks: Vec<_> = blocks.into_iter().flatten().collect();
    // The routes and topics API descriptions declare (docs/IMPORTS.md).
    blocks.extend(declare::blocks(&facts, &system));
    blocks.sort_by_key(|b| b.origin == model::Origin::Discovered);

    let rels: Vec<String> = files.iter().map(|f| f.rel.clone()).collect();
    let mut nodes = build::build(&system, blocks, &docs, &rels, &mut diags);
    declare::settle(&mut nodes, &system, &config, &mut diags);
    let (mut nodes, mut edges) = resolve::resolve(nodes, &system, &mut diags);
    request::link(&nodes, &mut edges, &mut diags);
    // After the request lines: a scenario can step along a request the code sends (docair-4wk.15).
    let flows = resolve::flow_paths(&nodes, &edges, &mut diags);
    if config.checks.as_ref().is_some_and(|c| c.unrequested_routes) {
        request::unrequested(&nodes, &edges, &mut diags);
    }
    declare::word_and_prune(&mut nodes, &mut edges);
    // Each line's id, now that the lines are final (docair-svh).
    ids::edge_ids(&mut edges);
    let reports = config
        .tests
        .as_ref()
        .map(|t| t.results.clone())
        .unwrap_or_default();
    test_results::attach(root, &reports, &mut nodes, &mut diags);
    let walkthroughs =
        walkthrough::load(root, config.walkthrough_dir(), &nodes, &edges, &mut diags);
    glossary::check(&terms, &nodes, &mut diags);
    let decisions = meanings::load(root, &config, &mut diags);
    let (terms, meanings) = meanings::settle(
        meanings::Input {
            terms,
            nodes: &nodes,
            edges: &edges,
            walkthroughs: &walkthroughs,
            docs: &md_files,
            decisions: &decisions,
        },
        &mut diags,
    );
    let library = glossary::library(root, &terms);
    let lenient = opts.lenient || config.strict == Some(false);
    let diagnostics = check::check(&nodes, &edges, diags, lenient);

    Ok(Graph {
        schema: SCHEMA.to_string(),
        system,
        generator: format!("docair {}", env!("CARGO_PKG_VERSION")),
        nodes,
        edges,
        flows,
        walkthroughs,
        view: View {
            hide: config
                .view
                .as_ref()
                .map(|v| v.hide.clone())
                .unwrap_or_default(),
            show: config
                .view
                .as_ref()
                .map(|v| v.show.clone())
                .unwrap_or_default(),
            tests: config.view.as_ref().and_then(|v| v.tests).unwrap_or(true),
        },
        glossary: terms,
        meanings: (!meanings.is_empty()).then_some(meanings),
        library,
        diagnostics,
    })
}
