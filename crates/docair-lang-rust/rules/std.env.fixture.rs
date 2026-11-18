// Fixture for std.env.yaml. Every `//>` line is a fact the rule must find,
// and it must find no others.

pub fn bind() -> String {
    std::env::var("DOCAIR_BIND").unwrap_or_default()
}

pub fn dir() -> Option<std::ffi::OsString> {
    env::var_os("DOCAIR_STATIC_DIR")
}

// A name that comes in: the adapter follows it, not the rule.
pub fn get(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

//> requests env:DOCAIR_BIND by fn bind
//> requests env:DOCAIR_STATIC_DIR by fn dir
