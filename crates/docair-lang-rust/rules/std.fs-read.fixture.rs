// Fixture for std.fs-read.yaml. Every `//>` line is a fact the rule must
// find, and it must find no others.

pub fn config(root: &Path) -> String {
    std::fs::read_to_string(root.join("docair.toml")).unwrap_or_default()
}

pub fn env_file() -> String {
    fs::read_to_string(".env").unwrap_or_default()
}

pub fn walks(dir: &Path) -> usize {
    fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

//> reads file: from fn config
//> reads file:.env from fn env_file
//> reads file: from fn walks
