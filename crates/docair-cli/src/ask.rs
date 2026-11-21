//! `docair ask`, `docair chat`, `docair apply`, `docair ai` (SPEC §10).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use docair_ai::{AiConfig, Env, Reply, Turn};
use docair_core::edit::PatchResult;
use docair_core::{Options, scan};
use docair_server::project::{Pending, ProjectSession, no_ai, read_pending};

/// What only the CLI does with a session: open it with the AI settings it asks with, and ask in the terminal.
trait ProjectSessionCli: Sized {
    fn open(root: &Path, out: Option<PathBuf>, lenient: bool, save: bool) -> Result<Option<Self>>;
    fn turn(&mut self, question: &str, history: &[Turn], apply: bool) -> Result<Turn>;
}

impl ProjectSessionCli for ProjectSession {
    fn open(
        root: &Path,
        out: Option<PathBuf>,
        lenient: bool,
        save: bool,
    ) -> Result<Option<ProjectSession>> {
        let Some(ai) = ai_config(root)? else {
            return Ok(None);
        };
        if ai.api_key.is_none() {
            eprintln!(
                "docair: no key in {} (environment or .env); asking without one",
                ai.key_var
            );
        }
        ProjectSession::load(root, out, lenient, save, ai).map(Some)
    }
    /***
     * One turn: ask, print the reply, record the walkthrough, write the patch; with `apply`, write edits and view.
     *
     * @id      docair-cli.ask.turn
     * @calls   docair-ai.ask.ask (question, graph, earlier turns) -> Reply
     * @calls   docair-core.walkthrough.save "records the walkthrough as JSON"
     * @calls   docair-core.edit.patch "writes the edits into the files"
     * @writes  project-files (edited doc blocks, docair.toml [view])
     ***/
    fn turn(&mut self, question: &str, history: &[Turn], apply: bool) -> Result<Turn> {
        eprintln!(
            "docair: asking {} about {} nodes and {} edges …",
            self.ai.model,
            self.graph.nodes.len(),
            self.graph.edges.len()
        );
        let reply = docair_ai::ask(&self.ai, &self.graph, question, history, None)?;
        print_reply(&reply, &self.ai);
        for p in &reply.problems {
            eprintln!("warning: {p} (left out)");
        }
        let (pending, result, replaced) = self.take_reply(&reply, history.is_empty())?;
        // An edit merged into the one already waiting for its part says so (docair-b7e).
        for node in &replaced {
            println!("replaced  the edit waiting for {node}: this reply's edit holds both");
        }
        if let Some(result) = result {
            print_patch(&result);
            println!("patch     {}", self.out.join("docair.patch").display());
        }
        if apply {
            self.apply(&pending)?;
            std::fs::remove_file(self.out.join("pending.json")).ok();
        } else if !pending.changes.is_empty() || pending.view.is_some() {
            println!("apply     docair apply (or `git apply .docair/docair.patch`)");
        }
        let html = self.write_outputs()?;
        println!("page      {}", html.display());
        Ok(Turn {
            question: question.to_string(),
            answer: reply.answer,
            context: Vec::new(),
        })
    }
}

/***
 * `docair word`: the model writes words for the lines that say nothing; they become edits of the tags, `--apply` writes them.
 *
 * @calls docair-ai.word.word (graph, scope) -> words + edits
 * @calls docair-core.edit.patch "writes the tag edits into files"
 ***/
pub fn cmd_word(
    root: &Path,
    out: Option<PathBuf>,
    lenient: bool,
    scope: Option<String>,
    limit: usize,
    all: bool,
    apply: bool,
) -> Result<ExitCode> {
    let Some(mut s) = ProjectSession::open(root, out, lenient, false)? else {
        return Ok(ExitCode::from(2));
    };
    let todo = docair_ai::word::unworded(&s.graph, scope.as_deref(), all).len();
    eprintln!(
        "docair: {todo} lines say nothing yet; asking {} for words on up to {limit} …",
        s.ai.model
    );
    let reply = docair_ai::word(&s.ai, &s.graph, scope.as_deref(), limit, all)?;
    println!();
    for w in &reply.wording {
        println!("  {} → {}: {}", w.from, w.to, w.text);
    }
    println!();
    println!("{}", wrap(&reply.answer, 0));
    println!(
        "  {} · {:.0} s · {} tokens in, {} out",
        s.ai.model, reply.seconds, reply.usage.input, reply.usage.output
    );
    for p in &reply.problems {
        eprintln!("warning: {p}");
    }
    let pending = Pending {
        changes: reply.changes.clone(),
        view: None,
    };
    std::fs::create_dir_all(&s.out)?;
    std::fs::write(
        s.out.join("pending.json"),
        serde_json::to_string_pretty(&pending)?,
    )?;
    let result = s.patch(&reply.changes);
    let patch = s.out.join("docair.patch");
    std::fs::write(&patch, &result.diff)?;
    println!(
        "patch     {} ({} files)",
        patch.display(),
        result.files.len()
    );
    if apply {
        s.apply(&pending)?;
        std::fs::remove_file(s.out.join("pending.json")).ok();
    } else {
        println!("apply     docair apply (or `git apply .docair/docair.patch`)");
    }
    let html = s.write_outputs()?;
    println!("page      {}", html.display());
    Ok(ExitCode::SUCCESS)
}

/***
 * `docair glossary`: the model explains the words of the docs that the glossary does not explain yet; `--apply` writes the entries.
 *
 * @calls docair-ai.glossary.explain (graph) -> new entries "asks for the missing terms"
 * @calls docair-core.edit.patch "turns the entries into a patch"
 ***/
pub fn cmd_glossary(
    root: &Path,
    out: Option<PathBuf>,
    lenient: bool,
    limit: usize,
    apply: bool,
    reasoning: Option<&str>,
) -> Result<ExitCode> {
    let Some(mut s) = ProjectSession::open(root, out, lenient, false)? else {
        return Ok(ExitCode::from(2));
    };
    // The flag, else docair.toml's [ai] glossary_reasoning, else low (docair-8l6.89).
    let reasoning = docair_ai::glossary::reasoning_of(
        reasoning.or(s
            .config
            .ai
            .as_ref()
            .and_then(|a| a.glossary_reasoning.as_deref())),
    )?;
    eprintln!(
        "docair: {} terms explained so far; asking {} for up to {limit} more, reasoning {reasoning} …",
        s.graph.glossary.len(),
        s.ai.model
    );
    let reply = docair_ai::glossary::explain(&s.ai, &s.graph, limit, reasoning)?;
    println!();
    for t in &reply.glossary {
        println!("{}", t.term);
        println!("{}", wrap(&t.definition, 2));
        if !t.see.is_empty() {
            println!("  see {}", t.see.join(", "));
        }
        if !t.link.is_empty() {
            println!("  {}", t.link);
        }
    }
    println!();
    println!("{}", wrap(&reply.answer, 0));
    for p in &reply.problems {
        eprintln!("warning: {p}");
    }
    let pending = Pending {
        changes: reply.changes.clone(),
        view: None,
    };
    std::fs::create_dir_all(&s.out)?;
    std::fs::write(
        s.out.join("pending.json"),
        serde_json::to_string_pretty(&pending)?,
    )?;
    let result = s.patch(&reply.changes);
    std::fs::write(s.out.join("docair.patch"), &result.diff)?;
    if apply {
        s.apply(&pending)?;
        std::fs::remove_file(s.out.join("pending.json")).ok();
    } else if !reply.changes.is_empty() {
        println!("apply     docair apply (or `git apply .docair/docair.patch`)");
    }
    let html = s.write_outputs()?;
    println!("page      {}", html.display());
    Ok(ExitCode::SUCCESS)
}

/// The AI settings for a project, or a hint how to turn AI on.
pub fn ai_config(root: &Path) -> Result<Option<AiConfig>> {
    let config = docair_core::config::load(root)?;
    let ai = AiConfig::resolve(config.ai.as_ref(), &Env::load(Some(root)));
    if ai.is_none() {
        eprintln!(
            "docair: AI is off. Set `endpoint` and `model` in [ai] of docair.toml, \
             or DOCAIR_AI_ENDPOINT and DOCAIR_AI_MODEL (SPEC §10.1)."
        );
    }
    Ok(ai)
}

fn conversation_path(out: &Path) -> PathBuf {
    out.join("conversation.json")
}

/***
 * `docair ask`: one question; with `--follow-up` it continues the last conversation, with `--apply` it writes the edits.
 *
 * @flow    ask 1 "you run docair ask with a question"
 * @creates data.question "you type it after docair ask"
 * @calls   docair-core.scan "builds the graph of the project"
 * @calls   docair-ai.config.resolve "combines the settings with the environment"
 * @calls   docair-cli.ask.turn "runs one question-answer cycle"
 * @calls   docair-viewer.write_html "renders the viewer page"
 ***/
pub fn cmd_ask(
    question: &str,
    root: &Path,
    out: Option<PathBuf>,
    lenient: bool,
    save: bool,
    follow_up: bool,
    apply: bool,
) -> Result<ExitCode> {
    let Some(mut s) = ProjectSession::open(root, out, lenient, save)? else {
        return Ok(ExitCode::from(2));
    };
    let path = conversation_path(&s.out);
    let mut history: Vec<Turn> = if follow_up {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let turn = s.turn(question, &history, apply)?;
    history.push(turn);
    std::fs::write(&path, serde_json::to_string_pretty(&history)?)?;
    Ok(ExitCode::SUCCESS)
}

/***
 * `docair chat`: a conversation in the terminal. Each line is a question; `/apply` writes the last reply's edits, `/quit` ends.
 *
 * @creates data.question "you type it, one line each"
 * @calls   docair-cli.ask.turn "runs one question-answer cycle"
 ***/
pub fn cmd_chat(root: &Path, out: Option<PathBuf>, lenient: bool) -> Result<ExitCode> {
    let Some(mut s) = ProjectSession::open(root, out, lenient, true)? else {
        return Ok(ExitCode::from(2));
    };
    println!(
        "docair chat · {} · {} nodes. Ask anything; /apply writes the last edits, /quit ends.",
        s.ai.model,
        s.graph.nodes.len()
    );
    let mut history: Vec<Turn> = Vec::new();
    let stdin = std::io::stdin();
    loop {
        print!("› ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        match line {
            "" => continue,
            "/quit" | "/exit" => break,
            "/apply" => {
                let pending = read_pending(&s.out)?;
                if pending.changes.is_empty() && pending.view.is_none() {
                    println!("nothing to apply");
                } else {
                    s.apply(&pending)?;
                    s.write_outputs()?;
                    std::fs::remove_file(s.out.join("pending.json")).ok();
                }
            }
            question => match s.turn(question, &history, false) {
                Ok(turn) => history.push(turn),
                Err(e) => eprintln!("docair: {e:#}"),
            },
        }
    }
    std::fs::write(
        conversation_path(&s.out),
        serde_json::to_string_pretty(&history)?,
    )?;
    Ok(ExitCode::SUCCESS)
}

/***
 * `docair apply`: write the edits and view change of the last reply into the files, then scan again.
 *
 * @calls docair-core.edit.resolve "locates where each edit goes"
 * @calls docair-core.edit.patch "writes the edits into the files"
 ***/
pub fn cmd_apply(root: &Path, out: Option<PathBuf>, lenient: bool) -> Result<ExitCode> {
    let out = out.unwrap_or_else(|| root.join(".docair"));
    let pending = read_pending(&out)?;
    let config = docair_core::config::load(root)?;
    let graph = scan(
        root,
        docair_adapters::all(),
        &Options {
            lenient,
            ..Options::default()
        },
    )?;
    let ai = AiConfig::resolve(config.ai.as_ref(), &Env::load(Some(root))).unwrap_or_else(no_ai);
    let mut s = ProjectSession {
        root: root.to_path_buf(),
        out,
        lenient,
        config,
        ai,
        graph,
        save: false,
        untrusted: false,
        changes: None,
    };
    s.apply(&pending)?;
    std::fs::remove_file(s.out.join("pending.json")).ok();
    let html = s.write_outputs()?;
    println!("page      {}", html.display());
    Ok(ExitCode::SUCCESS)
}

/// The reply as text: answer, walkthrough, view change, edits.
fn print_reply(reply: &Reply, ai: &AiConfig) {
    println!();
    println!("{}", wrap(&reply.answer, 0));
    println!(
        "  {} · {:.0} s · {} tokens in, {} out{}",
        ai.model,
        reply.seconds,
        reply.usage.input,
        reply.usage.output,
        if reply.requests > 1 {
            " · repaired once"
        } else {
            ""
        }
    );
    if let Some(wt) = &reply.walkthrough {
        println!();
        println!("✦ {}", wt.title);
        if !wt.summary.is_empty() {
            println!("{}", wrap(&wt.summary, 2));
        }
        for (k, s) in wt.steps.iter().enumerate() {
            println!();
            println!("{} {}", k + 1, s.title);
            if !s.text.is_empty() {
                println!("{}", wrap(&s.text, 2));
            }
            let edges: Vec<String> = s
                .edges
                .iter()
                .map(|e| format!("{} → {}", e.from, e.to))
                .collect();
            let refs: Vec<&str> = s.nodes.iter().map(String::as_str).collect();
            println!(
                "  · {}",
                if edges.is_empty() {
                    refs.join(", ")
                } else {
                    edges.join(", ")
                }
            );
        }
    }
    if let Some(v) = &reply.view {
        println!();
        if !v.hide.is_empty() {
            println!("view      hide {}", v.hide.join(", "));
        }
        if !v.show.is_empty() {
            println!("view      show {}", v.show.join(", "));
        }
    }
    for c in &reply.changes {
        println!();
        println!("✎ {} ({} {}:{})", c.node, c.mode, c.file, c.line);
        if !c.why.is_empty() {
            println!("{}", wrap(&c.why, 2));
        }
    }
    println!();
}

fn print_patch(result: &PatchResult) {
    for line in result.diff.lines() {
        println!("  {line}");
    }
    for p in &result.problems {
        eprintln!("warning: {p}");
    }
}

fn wrap(text: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = Vec::new();
    for para in text.split("\n\n") {
        let mut line = pad.clone();
        for word in para.split_whitespace() {
            if line.len() > indent && line.len() + 1 + word.len() > 80 {
                out.push(std::mem::replace(&mut line, pad.clone()));
            }
            if line.len() > indent {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out.join("\n")
}

/***
 * `docair ai`: show the AI settings and where the key comes from, never the key; `--check` sends one short request.
 *
 * @calls docair-ai.config.resolve "combines the settings with the environment"
 * @calls docair-ai.ask.check "sends one short request"
 ***/
pub fn cmd_ai(root: &Path, check: bool) -> Result<ExitCode> {
    let Some(ai) = ai_config(root)? else {
        return Ok(ExitCode::from(2));
    };
    println!("endpoint    {}", ai.public_endpoint());
    println!("model       {}", ai.model);
    match &ai.key_source {
        Some(at) => println!("key         set ({at})"),
        None => println!(
            "key         none: set {} in the environment or in .env",
            ai.key_var
        ),
    }
    println!("max_tokens  {}", ai.max_tokens);
    if check {
        let (reply, seconds) = docair_ai::check(&ai)?;
        println!("check       answered {reply:?} in {seconds:.1} s");
    }
    Ok(ExitCode::SUCCESS)
}
