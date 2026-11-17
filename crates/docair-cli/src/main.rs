//! The `docair` command line tool (SPEC §1.2).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use docair_core::model::{Graph, Severity};
use docair_core::{Options, scan};

mod ask;

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
    /// Run all stages; write docair.graph.json and docair.html.
    Scan {
        /// Project root.
        #[arg(default_value = ".")]
        root: PathBuf,
        /// Output directory [default: <root>/.docair].
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Report unresolved refs as warnings.
        #[arg(long)]
        lenient: bool,
    },
    /// Run stages 1–6 and print diagnostics; exits non-zero on errors.
    Check {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        lenient: bool,
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
 * @calls docair-cli.ask.cmd_word
 * @calls docair-cli.ask.cmd_glossary "labels the silent lines with the model"
 * @calls docair-cli.ask.cmd_ai "shows the model settings"
 ***/
fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Scan { root, out, lenient } => cmd_scan(&root, out, lenient),
        Command::Check { root, lenient } => cmd_check(&root, lenient),
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
 * @calls docair-cli.print_diagnostics "reports each diagnostic with its location"
 ***/
fn cmd_scan(root: &Path, out: Option<PathBuf>, lenient: bool) -> Result<ExitCode> {
    let graph = scan(root, docair_adapters::all(), &Options { lenient })?;
    let out = out.unwrap_or_else(|| root.join(".docair"));
    let json = out.join("docair.graph.json");
    let html = out.join("docair.html");
    docair_core::emit::write_json(&graph, &json)?;
    docair_viewer::write_html(&graph, &html)?;
    print_diagnostics(&graph);
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
fn cmd_check(root: &Path, lenient: bool) -> Result<ExitCode> {
    let graph = scan(root, docair_adapters::all(), &Options { lenient })?;
    print_diagnostics(&graph);
    let (errors, warnings) = counts(&graph);
    println!(
        "docair check: {} nodes, {} edges, {errors} errors, {warnings} warnings",
        graph.nodes.len(),
        graph.edges.len()
    );
    Ok(exit_code(&graph))
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
