//! A definition's SCIP symbol, as rust-analyzer names it.
//!
//! Copied from rust-analyzer's `ide/src/moniker.rs` (`def_to_moniker`, `def_to_non_local_moniker`,
//! `def_to_kind`, `display`; MIT or Apache-2.0), where they are crate-private, with the moniker
//! turned into a SCIP symbol at once as its `cli/scip.rs` does: so a file can be indexed without
//! rust-analyzer's static index of the whole workspace, under the same symbols.

use ra_ap_hir::{Adt, AsAssocItem, HirDisplay, MacroKind};
use ra_ap_ide::{MonikerDescriptorKind, SymbolInformationKind};
use ra_ap_ide_db::RootDatabase;
use ra_ap_ide_db::base_db::{CrateOrigin, LangCrateOrigin};
use ra_ap_ide_db::defs::Definition;
use scip::types as scip_types;

/// A definition's symbol: one of the whole project, or local to its file.
pub enum Sym {
    Global(String),
    Local,
}

/***
 * The symbol of a definition: local for locals, labels and generic parameters; none for what is in no module (built-in types and attributes, tuple fields, tool modules).
 ***/
pub fn symbol_of(db: &RootDatabase, def: Definition<'_>) -> Option<Sym> {
    match def {
        Definition::Local(_) | Definition::Label(_) | Definition::GenericParam(_) => {
            Some(Sym::Local)
        }
        _ => non_local(db, def).map(|s| Sym::Global(scip::symbol::format_symbol(s))),
    }
}

fn non_local(db: &RootDatabase, definition: Definition<'_>) -> Option<scip_types::Symbol> {
    let module = match definition {
        Definition::Module(module) if module.is_crate_root(db) => module,
        _ => definition.module(db)?,
    };
    let krate = module.krate(db);
    let edition = krate.edition(db);
    // Descriptors for this definition and every enclosing definition.
    let mut reverse: Vec<(String, MonikerDescriptorKind)> = vec![];
    let mut def = definition;
    loop {
        match def {
            Definition::SelfType(impl_) => {
                if let Some(trait_ref) = impl_.trait_ref(db) {
                    // Trait impls use the trait type for the 2nd parameter.
                    reverse.push((
                        display(db, module, trait_ref),
                        MonikerDescriptorKind::TypeParameter,
                    ));
                }
                // Both inherent and trait impls use the self type for the first parameter.
                reverse.push((
                    display(db, module, impl_.self_ty(db)),
                    MonikerDescriptorKind::TypeParameter,
                ));
                reverse.push(("impl".to_owned(), MonikerDescriptorKind::Type));
            }
            _ => {
                if let Some(name) = def.name(db) {
                    reverse.push((name.display(db, edition).to_string(), kind(db, def).into()));
                } else if let Definition::Module(m) = def
                    && m.is_crate_root(db)
                    && reverse.is_empty()
                {
                    // Only `crate` by itself: `rust-analyzer cargo foo . bar/`, not `… crate/bar/`.
                    reverse.push(("crate".to_owned(), MonikerDescriptorKind::Namespace));
                }
            }
        }
        let Some(next) = def.enclosing_definition(db) else {
            break;
        };
        def = next;
    }
    if reverse.is_empty() {
        return None;
    }
    reverse.reverse();
    let (name, version) = match krate.origin(db) {
        CrateOrigin::Library { name, .. } => (name.as_str().to_owned(), krate.version(db)),
        CrateOrigin::Local { name, .. } => (
            name.map(|n| n.as_str().to_owned())
                .unwrap_or(krate.display_name(db)?.canonical_name().as_str().to_owned()),
            krate.version(db),
        ),
        CrateOrigin::Rustc { name } => (
            name.as_str().to_owned(),
            Some(format!("https://github.com/rust-lang/rust/compiler/{name}")),
        ),
        CrateOrigin::Lang(lang) => (
            krate.display_name(db)?.canonical_name().as_str().to_owned(),
            Some(match lang {
                LangCrateOrigin::Other => "https://github.com/rust-lang/rust/library/".into(),
                lang => format!("https://github.com/rust-lang/rust/library/{lang}"),
            }),
        ),
    };
    use scip_types::descriptor::Suffix::*;
    Some(scip_types::Symbol {
        scheme: "rust-analyzer".into(),
        package: Some(scip_types::Package {
            manager: "cargo".to_owned(),
            name,
            version: version.unwrap_or_else(|| ".".to_owned()),
            special_fields: Default::default(),
        })
        .into(),
        descriptors: reverse
            .into_iter()
            .map(|(name, desc)| scip_types::Descriptor {
                name,
                disambiguator: String::new(),
                suffix: match desc {
                    MonikerDescriptorKind::Namespace => Namespace,
                    MonikerDescriptorKind::Type => Type,
                    MonikerDescriptorKind::Term => Term,
                    MonikerDescriptorKind::Method => Method,
                    MonikerDescriptorKind::TypeParameter => TypeParameter,
                    MonikerDescriptorKind::Parameter => Parameter,
                    MonikerDescriptorKind::Macro => Macro,
                    MonikerDescriptorKind::Meta => Meta,
                }
                .into(),
                special_fields: Default::default(),
            })
            .collect(),
        special_fields: Default::default(),
    })
}

fn display<'db, T: HirDisplay<'db>>(
    db: &'db RootDatabase,
    module: ra_ap_hir::Module,
    it: T,
) -> String {
    match it.display_source_code(db, module.into(), true) {
        Ok(result) => result,
        // The display that always works.
        Err(_) => it
            .display(db, module.krate(db).to_display_target(db))
            .to_string(),
    }
}

fn kind(db: &RootDatabase, def: Definition<'_>) -> SymbolInformationKind {
    use SymbolInformationKind::*;
    match def {
        Definition::Macro(it) => match it.kind(db) {
            MacroKind::Derive
            | MacroKind::DeriveBuiltIn
            | MacroKind::AttrBuiltIn
            | MacroKind::Attr => Attribute,
            MacroKind::Declarative | MacroKind::DeclarativeBuiltIn | MacroKind::ProcMacro => Macro,
        },
        Definition::Field(..) | Definition::TupleField(..) => Field,
        Definition::Module(..) | Definition::Crate(..) => Module,
        Definition::Function(it) => {
            if it.as_assoc_item(db).is_some() {
                if it.has_self_param(db) {
                    if it.has_body(db) { Method } else { TraitMethod }
                } else {
                    StaticMethod
                }
            } else {
                Function
            }
        }
        Definition::Adt(Adt::Struct(..)) => Struct,
        Definition::Adt(Adt::Union(..)) => Union,
        Definition::Adt(Adt::Enum(..)) => Enum,
        Definition::EnumVariant(..) => EnumMember,
        Definition::Const(..) => Constant,
        Definition::Static(..) => StaticVariable,
        Definition::Trait(..) => Trait,
        Definition::TypeAlias(it) => {
            if it.as_assoc_item(db).is_some() {
                AssociatedType
            } else {
                TypeAlias
            }
        }
        Definition::BuiltinType(..) => Type,
        Definition::BuiltinLifetime(_) => TypeParameter,
        Definition::SelfType(..) => TypeAlias,
        Definition::GenericParam(..) => TypeParameter,
        Definition::Local(it) => {
            if it.is_self(db) {
                SelfParameter
            } else if it.is_param(db) {
                Parameter
            } else {
                Variable
            }
        }
        Definition::Label(..) | Definition::InlineAsmOperand(_) => Variable,
        Definition::DeriveHelper(..) => Attribute,
        Definition::BuiltinAttr(..) => Attribute,
        Definition::ToolModule(..) => Module,
        Definition::ExternCrateDecl(..) => Module,
        Definition::InlineAsmRegOrRegClass(..) => Module,
    }
}
