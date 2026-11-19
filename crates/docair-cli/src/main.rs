//! The `docair` command line tool (SPEC §1.2).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use docair_core::model::{Graph, Severity};
use docair_core::{Options, scan};

mod ask;
mod diff;
mod serve;
mod tidy;

#[derive(Parser)]
#[command(
    name = "docair",
    version,
    about = "Doc blocks + Markdown + code → data-flow model → one HTML page"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build the documentation, on demand: run all stages, write docair.graph.json and docair.html.
    #[command(visible_alias = "scan")]
    Build {
        /// Project root.
        #[arg(default_value = ".")]
        root: PathBuf,
        /// Output directory [default: <root>/.docair].
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Report unresolved refs as warnings.
        #[arg(long)]
        lenient: bool,
        /// Add a changes section: what the working tree changes since this commit (docs/CHANGES.md).
        #[arg(long, value_name = "A")]
        changes: Option<String>,
    },
    /// What two commits change in the system's flows (docs/CHANGES.md); without B, the working tree is B.
    Diff {
        /// The commit to compare from.
        a: String,
        /// The commit to compare to [default: the working tree].
        b: Option<String>,
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Output directory for changes.json [default: <root>/.docair].
        #[arg(long, short)]
        out: Option<PathBuf>,
        #[arg(long)]
        lenient: bool,
        /// Print the changes as JSON instead of the summary.
        #[arg(long)]
        json: bool,
    },
    /// What two commits change, explained by the AI (docs/EXPLAIN.md): kept in [explain] store, a pull request comment on stdout.
    Explain {
        /// The commit to compare from.
        a: String,
        /// The commit to compare to [default: the working tree].
        b: Option<String>,
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        lenient: bool,
        /// Print the findings as JSON instead of the comment.
        #[arg(long)]
        json: bool,
    },
    /// Remove the hand-written tags that only repeat what the code says: list them, write the patch, `--apply` to write them.
    Tidy {
        #[arg(default_value = ".")]
        root: PathBuf,
        /// Output directory for docair.patch [default: <root>/.docair].
        #[arg(long, short)]
        out: Option<PathBuf>,
        #[arg(long)]
        lenient: bool,
        /// Write the edits into the files.
        #[arg(long)]
        apply: bool,
    },
    /// Run stages 1–6 and print diagnostics; exits non-zero on errors.
    Check {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        lenient: bool,
        /// Also print how many hand-written lines the code confirms (docs/ADAPTERS.md §9.5).
        #[arg(long)]
        coverage: bool,
    },
    /// Ask the AI model: how something works, what to hide, what is wrong in the docs.
    Ask {
        /// The question, e.g. "How does terraform provision the cluster?"
        question: String,
        /// Project root.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Output directory [default: <root>/.docair].
        #[arg(long, short)]
        out: Option<PathBuf>,
        #[arg(long)]
        lenient: bool,
        /// Print the walkthrough, but do not record it in the walkthroughs folder.
        #[arg(long)]
        no_save: bool,
        /// Continue the last conversation, so "it" and "that" keep their meaning.
        #[arg(long, short = 'f')]
        follow_up: bool,
        /// Write the proposed edits into the files and the view change into docair.toml.
        #[arg(long)]
        apply: bool,
    },
    /// A conversation with the AI model: one question per line; /apply, /quit.
    Chat {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, short)]
        out: Option<PathBuf>,
        #[arg(long)]
        lenient: bool,
    },
    /// Let the AI model write words for the lines that say nothing; they become edits of the tags.
    Word {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, short)]
        out: Option<PathBuf>,
        #[arg(long)]
        lenient: bool,
        /// Only lines that touch this node or anything inside it.
        #[arg(long)]
        scope: Option<String>,
        /// At most this many lines (40 per request).
        #[arg(long, default_value_t = 80)]
        limit: usize,
        /// Also lines that show only a type, such as `Order`: a phrase says what happens.
        #[arg(long)]
        all: bool,
        /// Write the words into the files.
        #[arg(long)]
        apply: bool,
    },
    /// Let the AI model explain the words of the docs that the glossary does not explain yet.
    Glossary {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, short)]
        out: Option<PathBuf>,
        #[arg(long)]
        lenient: bool,
        /// At most this many new terms.
        #[arg(long, default_value_t = 15)]
        limit: usize,
        /// Write the entries into the glossary.
        #[arg(long)]
        apply: bool,
    },
    /// Write the edits and view change of the last reply into the files.
    Apply {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, short)]
        out: Option<PathBuf>,
        #[arg(long)]
        lenient: bool,
    },
    /// Print a slice as JSON: one part of the system, from a seed to its ends; without a seed, list the seeds.
    Slice {
        /// A datum's or a part's id, `flow:<name>` or `walkthrough:<id>`.
        seed: Option<String>,
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        lenient: bool,
        /// Print every slice, as one JSON list.
        #[arg(long, conflicts_with = "seed")]
        all: bool,
        /// Also print the slice as a picture, laid out (docair-layout).
        #[arg(long)]
        layout: bool,
        /// Print the slice as a picture instead: an SVG, laid out and drawn by docair.
        #[arg(long, conflicts_with_all = ["all", "layout"])]
        svg: bool,
        /// Print the messages of each step instead, in words (with --all: of every slice with steps).
        #[arg(long, conflicts_with_all = ["layout", "svg"])]
        messages: bool,
    },
    /// Print what a layer draws as JSON; without a layer, list the layers the project offers.
    Layer {
        /// overview, infra, network, k8s, apps or code.
        layer: Option<String>,
        /// Open this box too (repeatable).
        #[arg(long)]
        open: Vec<String>,
        /// Show or leave out the tests [default: `[view] tests`].
        #[arg(long)]
        tests: Option<bool>,
        /// Print every layer the project offers, as one JSON list.
        #[arg(long, conflicts_with = "layer")]
        all: bool,
        /// Print the section of this box instead: the box at its layer, one level down, with its context.
        #[arg(long = "box", conflicts_with_all = ["layer", "all"])]
        section: Option<String>,
        /// Also lay the picture out: where each part goes (docair-layout).
        #[arg(long)]
        layout: bool,
        /// Print the picture instead as an SVG, laid out and drawn by docair.
        #[arg(long, conflicts_with_all = ["all", "layout", "measure"])]
        svg: bool,
        /// Lay out every layer and every slice, and print how long each took and how often its lines cross.
        #[arg(long, conflicts_with_all = ["layer", "all"])]
        measure: bool,
        /// With --measure: keep the layouts in this folder, and take the ones kept there before.
        #[arg(long, requires = "measure")]
        keep: Option<PathBuf>,
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        lenient: bool,
    },
    /// Serve the project's documentation and the AI on it, and uploads, at a local address; writes the documentation again when files change.
    Serve {
        #[arg(default_value = ".")]
        root: PathBuf,
        /// Output directory [default: <root>/.docair].
        #[arg(long, short)]
        out: Option<PathBuf>,
        #[arg(long)]
        lenient: bool,
        /// Address to listen on.
        #[arg(long, default_value = "127.0.0.1:8765")]
        bind: String,
        /// Serve the built web app from this folder at /app/.
        #[arg(long)]
        app: Option<PathBuf>,
        /// Add a changes section: what the working tree changes since this commit, written again with each change.
        #[arg(long, value_name = "A")]
        changes: Option<String>,
    },
    /// Show the AI settings and where the key comes from.
    Ai {
        #[arg(default_value = ".")]
        root: PathBuf,
        /// Send one short request to see that endpoint, model and key work.
        #[arg(long)]
        check: bool,
    },
}

/***
 * Parse the command line and run the chosen command.
 *
 * @calls docair-cli.cmd_scan "builds the graph and writes the outputs"
 * @calls docair-cli.cmd_check "reports the project's diagnostics"
 * @calls docair-cli.ask.cmd_ask "handles a one-shot question"
 * @calls docair-cli.ask.cmd_chat "starts a terminal conversation"
 * @calls docair-cli.ask.cmd_apply "writes the last reply's edits"
 * @calls docair-cli.ask.cmd_glossary "labels the silent lines with the model"
 * @calls docair-cli.ask.cmd_ai "shows the model settings"
 * @calls docair-cli.cmd_slice "prints a slice"
 * @calls docair-cli.cmd_layer "prints what a layer draws"
 * @calls docair-cli.serve.cmd_serve "serves the documentation and the AI"
 ***/
fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Build {
            root,
            out,
            lenient,
            changes,
        } => cmd_scan(&root, out, lenient, changes.as_deref()),
        Command::Tidy {
            root,
            out,
            lenient,
            apply,
        } => tidy::cmd_tidy(&root, out, lenient, apply),
        Command::Diff {
            a,
            b,
            root,
            out,
            lenient,
            json,
        } => diff::cmd_diff(&root, &a, b.as_deref(), out, lenient, json),
        Command::Explain {
            a,
            b,
            root,
            lenient,
            json,
        } => diff::cmd_explain(&root, &a, b.as_deref(), lenient, json),
        Command::Check {
            root,
            lenient,
            coverage,
        } => cmd_check(&root, lenient, coverage),
        Command::Ask {
            question,
            root,
            out,
            lenient,
            no_save,
            follow_up,
            apply,
        } => ask::cmd_ask(&question, &root, out, lenient, !no_save, follow_up, apply),
        Command::Chat { root, out, lenient } => ask::cmd_chat(&root, out, lenient),
        Command::Apply { root, out, lenient } => ask::cmd_apply(&root, out, lenient),
        Command::Glossary {
            root,
            out,
            lenient,
            limit,
            apply,
        } => ask::cmd_glossary(&root, out, lenient, limit, apply),
        Command::Word {
            root,
            out,
            lenient,
            scope,
            limit,
            all,
            apply,
        } => ask::cmd_word(&root, out, lenient, scope, limit, all, apply),
        Command::Slice {
            seed,
            root,
            lenient,
            all,
            layout,
            svg,
            messages,
        } => cmd_slice(seed.as_deref(), &root, lenient, all, layout, svg, messages),
        Command::Layer {
            layer,
            open,
            tests,
            all,
            section,
            layout,
            svg,
            measure,
            keep,
            root,
            lenient,
        } => {
            let shown = LayerOutput {
                all,
                section,
                layout,
                svg,
                measure,
                keep,
            };
            cmd_layer(layer.as_deref(), &open, tests, shown, &root, lenient)
        }
        Command::Serve {
            root,
            out,
            lenient,
            bind,
            app,
            changes,
        } => serve::cmd_serve(&root, out, lenient, &bind, app, changes),
        Command::Ai { root, check } => ask::cmd_ai(&root, check),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("docair: {e:#}");
            ExitCode::from(2)
        }
    }
}

/***
 * `docair scan`: build the graph, then write the JSON file and the HTML page.
 *
 * @flow  scan 1 "the user runs docair scan"
 * @calls docair-core.scan "builds the graph of the project"
 * @calls docair-core.emit.write_json "saves the graph for other tools"
 * @calls docair-viewer.write_html "renders the viewer page"
 * @calls docair-server.commits.compare "with --changes: compares the working tree with a commit"
 * @calls docair-server.commits.changes_page "with --changes: the page with its changes section"
 * @calls docair-cli.print_diagnostics "reports each diagnostic with its location"
 ***/
fn cmd_scan(
    root: &Path,
    out: Option<PathBuf>,
    lenient: bool,
    changes: Option<&str>,
) -> Result<ExitCode> {
    let out = out.unwrap_or_else(|| root.join(".docair"));
    let json = out.join("docair.graph.json");
    let html = out.join("docair.html");
    let graph = match changes {
        None => {
            let graph = scan(
                root,
                docair_adapters::all(),
                &Options {
                    lenient,
                    ..Options::default()
                },
            )?;
            docair_core::emit::write_json(&graph, &json)?;
            docair_viewer::write_html(&graph, &html)?;
            graph
        }
        Some(a) => {
            // The working tree compared with commit A: the page gets a changes section, with the
            // findings `docair explain` stored for this pair, when there are any.
            let pair = docair_server::commits::compare(root, a, None, lenient)?;
            let page = docair_server::commits::changes_page(&pair);
            std::fs::create_dir_all(&out)?;
            docair_core::emit::write_json(&pair.b, &json)?;
            std::fs::write(&html, page)?;
            std::fs::write(
                out.join("changes.json"),
                serde_json::to_string_pretty(&pair.changes)? + "\n",
            )?;
            println!(
                "docair: {} changed flows since {a} in the page's changes section",
                pair.changes.flows.len()
            );
            pair.b
        }
    };
    print_diagnostics(&graph);
    if let Some(h) = tidy::hint(&graph) {
        println!("{h}");
    }
    println!(
        "docair: {} nodes, {} edges, {} flows → {} and {}",
        graph.nodes.len(),
        graph.edges.len(),
        graph.flows.len(),
        json.display(),
        html.display()
    );
    Ok(exit_code(&graph))
}

/***
 * `docair check`: build the graph and report its diagnostics, for CI.
 *
 * @flow  check 1 "the user runs docair check"
 * @calls docair-core.scan "builds the graph of the project"
 * @calls docair-cli.print_diagnostics "reports each diagnostic with its location"
 ***/
fn cmd_check(root: &Path, lenient: bool, coverage: bool) -> Result<ExitCode> {
    let graph = scan(
        root,
        docair_adapters::all(),
        &Options {
            lenient,
            ..Options::default()
        },
    )?;
    print_diagnostics(&graph);
    if coverage {
        print_coverage(&graph);
    }
    if let Some(h) = tidy::hint(&graph) {
        println!("{h}");
    }
    let (errors, warnings) = counts(&graph);
    println!(
        "docair check: {} nodes, {} edges, {errors} errors, {warnings} warnings",
        graph.nodes.len(),
        graph.edges.len()
    );
    Ok(exit_code(&graph))
}

/***
 * `docair slice`: print one slice as JSON, or list the seeds a slice can start from.
 *
 * @calls docair-core.scan "builds the graph of the project"
 * @calls docair-core.slice.find "finds the seed the reader names"
 * @calls docair-core.slice.slice "works out the slice"
 * @calls docair-core.slice.seeds "lists the seeds"
 * @calls docair-core.slice.all "works out every slice"
 * @calls docair-core.layer_view.slice_picture "makes the slice a picture"
 * @calls docair-layout.place "lays the picture out"
 * @calls docair-viewer.svg.standalone "draws the picture as an SVG"
 ***/
#[allow(clippy::too_many_arguments)]
fn cmd_slice(
    seed: Option<&str>,
    root: &Path,
    lenient: bool,
    all: bool,
    layout: bool,
    svg: bool,
    messages: bool,
) -> Result<ExitCode> {
    let graph = scan(
        root,
        docair_adapters::all(),
        &Options {
            lenient,
            ..Options::default()
        },
    )?;
    if all && messages {
        let each: Vec<serde_json::Value> = docair_core::slice::all(&graph)
            .iter()
            .filter(|s| !s.steps.is_empty())
            .map(|s| {
                serde_json::json!({ "seed": s.seed, "messages": docair_core::slice::messages(&graph, s) })
            })
            .collect();
        println!("{}", docair_core::slice::to_json(&each));
        return Ok(ExitCode::SUCCESS);
    }
    if all {
        println!(
            "{}",
            docair_core::slice::to_json(&docair_core::slice::all(&graph))
        );
        return Ok(ExitCode::SUCCESS);
    }
    let Some(name) = seed else {
        for s in docair_core::slice::seeds(&graph) {
            let kind = serde_json::to_value(s.kind)?;
            println!("{:<12} {}", kind.as_str().unwrap_or_default(), s.id);
        }
        return Ok(ExitCode::SUCCESS);
    };
    let Some(seed) = docair_core::slice::find(&graph, name) else {
        eprintln!(
            "docair: no datum, part, flow or walkthrough named {name}; `docair slice` lists them"
        );
        return Ok(ExitCode::from(1));
    };
    let Some(slice) = docair_core::slice::slice(&graph, &seed) else {
        eprintln!("docair: {name} has no slice");
        return Ok(ExitCode::from(1));
    };
    if svg {
        let picture = docair_core::layer_view::slice_picture(&graph, &slice);
        let laid = docair_viewer::layout_input::lay_out(&graph, &picture);
        print!(
            "{}",
            docair_viewer::svg::standalone(&graph, &picture, &laid)
        );
        return Ok(ExitCode::SUCCESS);
    }
    if messages {
        println!(
            "{}",
            docair_core::slice::to_json(&docair_core::slice::messages(&graph, &slice))
        );
        return Ok(ExitCode::SUCCESS);
    }
    if layout {
        let picture = docair_core::layer_view::slice_picture(&graph, &slice);
        let font = docair_layout::Monospace::default();
        let input = docair_viewer::layout_input::input(&graph, &picture, &font);
        let laid =
            docair_layout::lay_out(&input, &docair_viewer::layout_input::options_for(&picture));
        let out = serde_json::json!({
            "slice": slice, "picture": picture, "input": input, "layout": laid.placement, "routes": laid.routes
        });
        println!("{}", docair_core::slice::to_json(&out));
        return Ok(ExitCode::SUCCESS);
    }
    println!("{}", docair_core::slice::to_json(&slice));
    Ok(ExitCode::SUCCESS)
}

/// What `docair layer` prints besides one layer's picture.
struct LayerOutput {
    all: bool,
    /// The section of this box instead of a layer.
    section: Option<String>,
    layout: bool,
    /// The picture as an SVG instead of JSON.
    svg: bool,
    measure: bool,
    keep: Option<PathBuf>,
}

/***
 * `docair layer`: print what one layer draws as JSON, every layer with `--all`, or list the layers offered.
 *
 * @calls docair-core.scan "builds the graph of the project"
 * @calls docair-core.layer_view.layers "lists the layers the project offers"
 * @calls docair-core.layer_view.picture "works out what a layer draws"
 * @calls docair-layout.lay_out "lays a picture out"
 * @calls docair-cli.print_measures "times every layout"
 * @calls docair-viewer.svg.standalone "draws the picture as an SVG"
 ***/
fn cmd_layer(
    layer: Option<&str>,
    open: &[String],
    tests: Option<bool>,
    shown: LayerOutput,
    root: &Path,
    lenient: bool,
) -> Result<ExitCode> {
    use docair_core::layer_view::{LayerKey, LayerPicture, layers, picture, to_json};
    let LayerOutput {
        all,
        section,
        layout,
        svg,
        measure,
        keep,
    } = shown;
    let laid = |graph: &Graph, p: LayerPicture| -> serde_json::Value {
        if layout {
            let font = docair_layout::Monospace::default();
            let input = docair_viewer::layout_input::input(graph, &p, &font);
            let laid =
                docair_layout::lay_out(&input, &docair_viewer::layout_input::options_for(&p));
            serde_json::json!({ "picture": p, "input": input, "layout": laid.placement, "routes": laid.routes })
        } else {
            serde_json::to_value(p).unwrap_or_default()
        }
    };
    let graph = scan(
        root,
        docair_adapters::all(),
        &Options {
            lenient,
            ..Options::default()
        },
    )?;
    if measure {
        print_measures(&graph, keep.as_deref())?;
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(bx) = section {
        let Some((_, p)) = docair_core::layer_view::section_picture(&graph, &bx, tests) else {
            eprintln!("docair: {bx} is no box that a layer draws");
            return Ok(ExitCode::from(1));
        };
        if svg {
            let laid = docair_viewer::layout_input::lay_out(&graph, &p);
            print!("{}", docair_viewer::svg::standalone(&graph, &p, &laid));
            return Ok(ExitCode::SUCCESS);
        }
        println!("{}", to_json(&laid(&graph, p)));
        return Ok(ExitCode::SUCCESS);
    }
    if all {
        let pictures: Vec<_> = layers(&graph)
            .iter()
            .filter_map(|l| picture(&graph, l.key, &[], tests))
            .map(|p| laid(&graph, p))
            .collect();
        println!("{}", to_json(&pictures));
        return Ok(ExitCode::SUCCESS);
    }
    let Some(name) = layer else {
        for l in layers(&graph) {
            let key = serde_json::to_value(l.key)?;
            println!(
                "{:<9} {:<15} {}",
                key.as_str().unwrap_or_default(),
                l.name,
                l.ask
            );
        }
        return Ok(ExitCode::SUCCESS);
    };
    let key: LayerKey = match serde_json::from_value(serde_json::Value::String(name.to_string())) {
        Ok(key) => key,
        Err(_) => {
            eprintln!(
                "docair: no layer {name}; the layers are overview, infra, network, k8s, apps, code"
            );
            return Ok(ExitCode::from(1));
        }
    };
    let Some(p) = picture(&graph, key, open, tests) else {
        eprintln!(
            "docair: this project has nothing for the {name} layer; `docair layer` lists the layers it offers"
        );
        return Ok(ExitCode::from(1));
    };
    if svg {
        let laid = docair_viewer::layout_input::lay_out(&graph, &p);
        print!("{}", docair_viewer::svg::standalone(&graph, &p, &laid));
        return Ok(ExitCode::SUCCESS);
    }
    println!("{}", to_json(&laid(&graph, p)));
    Ok(ExitCode::SUCCESS)
}

/***
 * Lay out every layer and every slice of the graph, and print, for each, its size, how often its lines cross drawn straight, and how long it took.
 *
 * @calls docair-layout.lay_out "lays each picture out"
 * @calls docair-viewer.layout_store.lay_out_kept "takes a kept layout, or keeps a new one"
 ***/
fn print_measures(graph: &Graph, keep: Option<&Path>) -> Result<()> {
    use docair_core::layer_view::{layers, picture, slice_picture};
    use docair_layout::Monospace;
    use std::time::Instant;
    let font = Monospace::default();
    let mut pictures = Vec::new();
    for l in layers(graph) {
        if let Some(p) = picture(graph, l.key, &[], None) {
            pictures.push((format!("layer {}", l.name), p));
        }
    }
    let boxes: std::collections::BTreeSet<&str> = graph
        .nodes
        .iter()
        .filter_map(|n| n.part_of.as_deref())
        .collect();
    for b in boxes {
        if let Some((_, p)) = docair_core::layer_view::section_picture(graph, b, None) {
            pictures.push((format!("section {b}"), p));
        }
    }
    for s in docair_core::slice::all(graph) {
        pictures.push((format!("slice {}", s.seed.id), slice_picture(graph, &s)));
    }
    println!(
        "{:<44} {:>6} {:>6} {:>9} {:>7} {:>5} {:>5} {:>9}",
        "picture", "parts", "lines", "crossings", "bends", "none", "dir", "ms"
    );
    let (mut total, mut worst, mut worst_name) = (0.0f64, 0.0f64, String::new());
    let mut slices = (0usize, 0.0f64, 0.0f64);
    let mut sections = (0usize, 0.0f64, 0.0f64);
    let mut sum_cross = 0usize;
    let mut kept = 0usize;
    for (name, p) in &pictures {
        let input = docair_viewer::layout_input::input(graph, p, &font);
        let t = Instant::now();
        let laid = match keep {
            Some(dir) => {
                let (laid, was_kept) = docair_viewer::layout_store::lay_out_kept(graph, p, dir)?;
                kept += usize::from(was_kept);
                laid
            }
            None => docair_layout::lay_out(&input, &docair_viewer::layout_input::options_for(p)),
        };
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let placed = &laid.placement;
        let crossings = docair_layout::crossings(&laid.routes);
        let routed = laid
            .routes
            .lines
            .iter()
            .filter(|r| r.len() >= 2)
            .count()
            .max(1);
        let bends = laid
            .routes
            .lines
            .iter()
            .map(|r| r.len().saturating_sub(2))
            .sum::<usize>() as f64
            / routed as f64;
        sum_cross += crossings;
        total += ms;
        if ms > worst {
            worst = ms;
            worst_name = name.clone();
        }
        if name.starts_with("section") {
            sections.0 += 1;
            sections.1 += ms;
            sections.2 = sections.2.max(ms);
        }
        if name.starts_with("slice") {
            slices.0 += 1;
            slices.1 += ms;
            slices.2 = slices.2.max(ms);
        }
        if name.starts_with("layer") || ms >= 20.0 {
            let dir = format!("{:?}", placed.direction).to_lowercase();
            println!(
                "{:<44} {:>6} {:>6} {:>9} {:>7.1} {:>5} {:>5} {:>9.1}",
                clip_name(name, 44),
                p.parts.len(),
                p.lines.len(),
                crossings,
                bends,
                laid.routes.none,
                dir,
                ms
            );
        }
    }
    println!(
        "{} pictures in {:.0} ms, {sum_cross} crossings in all; {} slices in {:.0} ms, {:.1} ms at most; the slowest: {worst_name} ({worst:.0} ms)",
        pictures.len(),
        total,
        slices.0,
        slices.1,
        slices.2
    );
    println!(
        "{} sections in {:.0} ms, {:.1} ms at most",
        sections.0, sections.1, sections.2
    );
    if keep.is_some() {
        println!("{kept} of {} layouts were kept from before", pictures.len());
    }
    Ok(())
}

fn clip_name(name: &str, max: usize) -> String {
    if name.chars().count() > max {
        let mut out: String = name.chars().take(max - 1).collect();
        out.push('…');
        out
    } else {
        name.to_string()
    }
}

/***
 * Print, per kind of line, how many hand-written lines the code confirms and how many lines only the code knows.
 *
 * @calls docair-core.coverage.coverage "counts the lines"
 ***/
fn print_coverage(graph: &Graph) {
    println!("lines      written  confirmed  via helpers  by a request  missing  only in code");
    let (mut written, mut accounted) = (0, 0);
    for (verb, c) in docair_core::coverage::coverage(graph) {
        println!(
            "@{verb:<9} {:>7}  {:>9}  {:>11}  {:>12}  {:>7}  {:>12}",
            c.authored,
            c.confirmed,
            c.via_helpers,
            c.by_request,
            c.missing(),
            c.code_only
        );
        written += c.authored;
        accounted += c.authored - c.missing();
    }
    let share = (accounted * 100).checked_div(written).unwrap_or(0);
    println!("the code accounts for {accounted} of {written} hand-written lines ({share}%)");
    print_data_coverage(graph);
}

/***
 * Print, for each datum, how many of the lines that carry it its slice reaches from where it is made.
 *
 * @calls docair-core.slice.slice "works out the datum's slice"
 ***/
fn print_data_coverage(graph: &Graph) {
    use docair_core::slice::{SeedKind, seeds, slice};
    let data: Vec<_> = seeds(graph)
        .into_iter()
        .filter(|s| s.kind == SeedKind::Data)
        .collect();
    if data.is_empty() {
        return;
    }
    println!("data                 carried by  reached  stretches of their own");
    for seed in data {
        let Some(c) = slice(graph, &seed).and_then(|s| s.covers) else {
            continue;
        };
        println!(
            "{:<20} {:>10}  {:>7}  {:>22}",
            seed.id, c.lines, c.reached, c.apart
        );
    }
}

/***
 * Print diagnostics as `error[E001]: message --> file:line`.
 *
 * @flow check 4 "diagnostics are printed, the exit code is set"
 * @in   diagnostics: Diagnostic[] from docair-core.check.check
 ***/
fn print_diagnostics(graph: &Graph) {
    for d in &graph.diagnostics {
        let level = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let at = match (&d.file, d.line) {
            (Some(f), Some(l)) => format!(" --> {f}:{l}"),
            (Some(f), None) => format!(" --> {f}"),
            _ => String::new(),
        };
        eprintln!("{level}[{}]: {}{at}", d.code, d.message);
    }
}

fn counts(graph: &Graph) -> (usize, usize) {
    let errors = graph
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    (errors, graph.diagnostics.len() - errors)
}

fn exit_code(graph: &Graph) -> ExitCode {
    if counts(graph).0 > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
