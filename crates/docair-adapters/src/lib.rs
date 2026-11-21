//! The adapter registry (docs/ADAPTERS.md §5.3).

use std::sync::OnceLock;

use docair_core::adapter::Adapter;

/// Every `docair-lang-*` dependency's adapter, as Cargo.toml lists them (build.rs).
static LISTED: &[&dyn Adapter] = &include!(concat!(env!("OUT_DIR"), "/adapters.rs"));

/***
 * Every adapter, ordered by its kind: code first, then infrastructure, API descriptions, and the generic guesses last; within a kind as listed.
 *
 * When two adapters know the same line, the first one says what it is. The
 * first call also registers the file types the adapters bring with core's
 * language table, so their doc blocks are read (docs/ADAPTERS.md §5.3).
 ***/
pub fn all() -> &'static [&'static dyn Adapter] {
    static ALL: OnceLock<Vec<&'static dyn Adapter>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut all = LISTED.to_vec();
        all.sort_by_key(|a| a.kind());
        for a in &all {
            docair_core::lang::register(a.languages());
        }
        all
    })
}

#[cfg(test)]
mod tests {
    use docair_core::adapter::AdapterKind;

    /// Every adapter has a name of its own, and they come by kind (docs/ADAPTERS.md §5.3).
    #[test]
    fn the_adapters_come_by_kind() {
        let all = super::all();
        let names: Vec<&str> = all.iter().map(|a| a.name()).collect();
        for name in [
            "rust",
            "typescript",
            "terraform",
            "helm",
            "k8s",
            "apis",
            "generic",
        ] {
            assert!(names.contains(&name), "{name} in {names:?}");
        }
        let kinds: Vec<AdapterKind> = all.iter().map(|a| a.kind()).collect();
        let mut sorted = kinds.clone();
        sorted.sort();
        assert_eq!(kinds, sorted, "{names:?}");
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        // A file type an adapter brings is read once the registry ran: core's table lacks it.
        assert_eq!(
            docair_core::lang::language_for_file("main.tf").map(|l| l.name),
            Some("terraform")
        );
        assert_eq!(
            docair_core::lang::language_for_file("_helpers.tpl").map(|l| l.name),
            Some("helm")
        );
    }
}
