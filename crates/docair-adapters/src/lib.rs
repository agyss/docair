//! The adapter registry (docs/ADAPTERS.md §5.3).

use docair_core::adapter::Adapter;
use docair_lang_apis::ApiAdapter;
use docair_lang_generic::GenericAdapter;
use docair_lang_infra::InfraAdapter;
use docair_lang_rust::RustAdapter;

static ALL: [&dyn Adapter; 4] = [&RustAdapter, &InfraAdapter, &ApiAdapter, &GenericAdapter];

/***
 * Every adapter, in order: language adapters first, the generic guesses last.
 *
 * When two adapters know the same line, the first one says what it is.
 ***/
pub fn all() -> &'static [&'static dyn Adapter] {
    &ALL
}
