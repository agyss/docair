//! The language registry: which comment syntax a file uses (SPEC §2).
//! Adding a language with a known comment family means adding one row.

/// How doc blocks are written in a family of languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    CStyle,
    Hash,
    Markup,
    /// Markdown is read differently, see SPEC §5.
    Markdown,
    /// Data without comments, such as JSON: no doc blocks, only adapters read it.
    Data,
}

/// Opening marker, closing marker, and the line prefix stripped from content lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub open: &'static str,
    pub close: &'static str,
    pub prefix: Option<char>,
}

const C_STYLE: Marker = Marker {
    open: "/***",
    close: "***/",
    prefix: Some('*'),
};
const HELM: Marker = Marker {
    open: "{{/***",
    close: "***/}}",
    prefix: Some('*'),
};
const HASH: Marker = Marker {
    open: "#***",
    close: "#***",
    prefix: Some('#'),
};
const MARKUP: Marker = Marker {
    open: "<!--***",
    close: "***-->",
    prefix: None,
};

impl Family {
    pub fn markers(self) -> &'static [Marker] {
        match self {
            Family::CStyle => &[HELM, C_STYLE],
            Family::Hash => &[HELM, HASH],
            Family::Markup => &[MARKUP],
            Family::Markdown | Family::Data => &[],
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct LanguageSpec {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub family: Family,
}

/***
 * The language registry: file extension → comment family.
 *
 * Rust and TypeScript/JS use the C-style marker, YAML/TOML/shell the hash
 * marker, HTML the markup marker. Markdown is listed so that stage 1 picks it
 * up. An adapter brings the file types only it reads (Terraform's `.tf`,
 * Helm's `.tpl`), registered by the adapter registry (`register`).
 ***/
pub static LANGUAGES: &[LanguageSpec] = &[
    LanguageSpec {
        name: "rust",
        extensions: &["rs"],
        family: Family::CStyle,
    },
    LanguageSpec {
        name: "typescript",
        extensions: &["ts", "tsx", "js", "jsx", "mjs", "cjs"],
        family: Family::CStyle,
    },
    LanguageSpec {
        name: "html",
        extensions: &["html", "htm"],
        family: Family::Markup,
    },
    LanguageSpec {
        name: "yaml",
        extensions: &["yaml", "yml"],
        family: Family::Hash,
    },
    LanguageSpec {
        name: "toml",
        extensions: &["toml"],
        family: Family::Hash,
    },
    LanguageSpec {
        name: "shell",
        extensions: &["sh", "bash"],
        family: Family::Hash,
    },
    LanguageSpec {
        name: "nginx",
        extensions: &["conf"],
        family: Family::Hash,
    },
    LanguageSpec {
        name: "dockerfile",
        extensions: &["dockerfile"],
        family: Family::Hash,
    },
    LanguageSpec {
        name: "markdown",
        extensions: &["md"],
        family: Family::Markdown,
    },
    // Read only when named like an API description, or listed in `[import] apis` (discover.rs).
    LanguageSpec {
        name: "json",
        extensions: &["json"],
        family: Family::Data,
    },
];

/// The registry row for a file, by name (`Dockerfile`, `Containerfile`,
/// `Dockerfile.web`) or else by extension.
pub fn language_for_file(file_name: &str) -> Option<&'static LanguageSpec> {
    let lower = file_name.to_ascii_lowercase();
    if lower == "dockerfile" || lower == "containerfile" || lower.starts_with("dockerfile.") {
        return language_for("dockerfile");
    }
    lower
        .rsplit_once('.')
        .and_then(|(_, ext)| language_for(ext))
}

/// The languages adapters bring besides the table (docs/ADAPTERS.md §5.3, docair-4wk.42).
static ADDED: std::sync::RwLock<Vec<&'static LanguageSpec>> = std::sync::RwLock::new(Vec::new());

/// Read the languages an adapter brings too: a file type core does not know is read from now on,
/// its doc blocks with its comment family. One already known by name is kept as it is.
pub fn register(specs: &'static [LanguageSpec]) {
    let mut added = ADDED.write().unwrap_or_else(|e| e.into_inner());
    for s in specs {
        if !LANGUAGES
            .iter()
            .chain(added.iter().copied())
            .any(|l| l.name == s.name)
        {
            added.push(s);
        }
    }
}

/// The registry row for a file extension, if docair reads that language.
pub fn language_for(extension: &str) -> Option<&'static LanguageSpec> {
    let ext = extension.to_ascii_lowercase();
    LANGUAGES
        .iter()
        .find(|l| l.extensions.contains(&ext.as_str()))
        .or_else(|| {
            let added = ADDED.read().unwrap_or_else(|e| e.into_inner());
            added
                .iter()
                .copied()
                .find(|l| l.extensions.contains(&ext.as_str()))
        })
}
