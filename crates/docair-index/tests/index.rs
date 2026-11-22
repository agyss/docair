//! docair-index on a small workspace of its own: the whole index, the lines of code a derive
//! generates, and a short run of one file giving that file's part of the whole index.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use protobuf::Message;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A workspace: `app`, whose `S` carries `#[derive(Hello)]` from `hello-derive`; the derive's
/// generated `hello` calls `app::greet`.
fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(
        r,
        "Cargo.toml",
        "[workspace]\nresolver = \"2\"\nmembers = [\"app\", \"hello-derive\"]\n",
    );
    write(
        r,
        "hello-derive/Cargo.toml",
        "[package]\nname = \"hello-derive\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\nproc-macro = true\n",
    );
    write(
        r,
        "hello-derive/src/lib.rs",
        r#"use proc_macro::{TokenStream, TokenTree};

#[proc_macro_derive(Hello)]
pub fn derive_hello(input: TokenStream) -> TokenStream {
    let mut it = input.into_iter();
    let mut name = String::new();
    while let Some(t) = it.next() {
        if let TokenTree::Ident(i) = &t
            && i.to_string() == "struct"
            && let Some(TokenTree::Ident(n)) = it.next()
        {
            name = n.to_string();
            break;
        }
    }
    format!("impl crate::Hello for {name} {{ fn hello(&self) {{ crate::greet() }} }}").parse().unwrap()
}
"#,
    );
    write(
        r,
        "app/Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nhello-derive = { path = \"../hello-derive\" }\n",
    );
    write(
        r,
        "app/src/main.rs",
        r#"pub trait Hello {
    fn hello(&self);
}

pub fn greet() {}

#[derive(hello_derive::Hello)]
pub struct S;

pub fn c5() {
    S.hello()
}

fn main() {
    c5();
}
"#,
    );
    let ok = Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(r)
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    dir
}

/// Every occurrence of an index file: (document, line 1-based, symbol, definition).
fn occurrences(path: &Path) -> BTreeSet<(String, i32, String, bool)> {
    let index = scip::types::Index::parse_from_bytes(&std::fs::read(path).unwrap()).unwrap();
    index
        .documents
        .into_iter()
        .flat_map(|d| {
            let doc = d.relative_path.clone();
            d.occurrences.into_iter().map(move |o| {
                (
                    doc.clone(),
                    o.range[0] + 1,
                    o.symbol,
                    o.symbol_roles & 1 == 1,
                )
            })
        })
        .collect()
}

fn run(args: &[&str]) -> std::process::Output {
    let out = Command::new(env!("CARGO_BIN_EXE_docair-index"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

#[test]
fn the_whole_index_draws_derives_and_a_short_run_gives_the_same_file() {
    let dir = workspace();
    let root = dir.path().to_str().unwrap();
    let full = dir.path().join("full.scip");
    let signals = dir.path().join("full.signals.json");
    run(&[
        "full",
        "--root",
        root,
        "--out",
        full.to_str().unwrap(),
        "--signals",
        signals.to_str().unwrap(),
    ]);
    let all = occurrences(&full);
    let main: BTreeSet<_> = all
        .iter()
        .filter(|o| o.0 == "app/src/main.rs")
        .cloned()
        .collect();
    let at_s: Vec<&(String, i32, String, bool)> = main.iter().filter(|o| o.1 == 8).collect();
    // The derive's `hello` is defined at `S`, and its call to `greet` comes from `S`.
    assert!(
        at_s.iter()
            .any(|o| o.3 && o.2.ends_with("impl#[S][Hello]hello().")),
        "a definition of the derived hello at S: {at_s:?}"
    );
    assert!(
        at_s.iter().any(|o| !o.3 && o.2.ends_with(" greet().")),
        "a call to greet from S: {at_s:?}"
    );
    // c5's call reaches the derived method.
    assert!(
        main.iter()
            .any(|o| o.1 == 11 && !o.3 && o.2.ends_with("impl#[S][Hello]hello()."))
    );
    assert!(
        std::fs::read_to_string(&signals)
            .unwrap()
            .contains("app/src/main.rs|c5")
    );

    let short = dir.path().join("short.scip");
    let out = run(&[
        "files",
        "--root",
        root,
        "--out",
        short.to_str().unwrap(),
        "--files",
        "app/src/main.rs",
        "--signals-base",
        signals.to_str().unwrap(),
    ]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(report["texts"]["app/src/main.rs"].is_string(), "{report}");
    assert_eq!(report["guard"], "kept", "{report}");
    assert_eq!(
        occurrences(&short),
        main,
        "the short run's file is the whole index's"
    );
}

#[test]
fn the_version_names_rust_analyzers_crates() {
    let out = run(&["--version"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("ra_ap 0.0.358"));
}
