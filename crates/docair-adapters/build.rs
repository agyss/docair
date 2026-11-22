//! The adapters, found by name (docs/ADAPTERS.md §5.3): every dependency of this crate named
//! `docair-lang-*` is one, and exports `ADAPTER`. Adding an adapter needs one line in Cargo.toml.

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let manifest = std::fs::read_to_string("Cargo.toml").expect("docair-adapters' Cargo.toml");
    let mut deps = false;
    let mut names = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            deps = line == "[dependencies]";
            continue;
        }
        let name = line.split(['=', '.', ' ']).next().unwrap_or("");
        if deps && name.starts_with("docair-lang-") {
            names.push(name.replace('-', "_"));
        }
    }
    let list: String = names
        .iter()
        .map(|n| format!("    {n}::ADAPTER,\n"))
        .collect();
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("adapters.rs");
    std::fs::write(out, format!("[\n{list}]\n")).expect("writes the list of adapters");
}
