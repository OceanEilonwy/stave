//! Every entry a kind's base fixes is found once, when the kind is
//! resolved, and points at the param or field of that name
//! ([`stave_model::defs::KindDef::base_entry`]).

use std::path::Path;

use stave_lang::ast::BaseKind;
use stave_model::defs::BaseEntry;
use stave_model::resolve;
use stave_modules::ModuleSet;
use stave_schema::kinds::{BaseField, base};

#[test]
fn base_entries_point_at_the_param_or_field_of_their_name() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet");
    let modules = ModuleSet::load(&folder).unwrap();
    let (defs, _) = resolve(&modules);
    let mut fasteners = 0;
    for kind in &defs.kinds {
        for field in BaseField::ALL {
            let word = field.word();
            let param = kind.params.iter().position(|p| p.decl.name == word);
            let written = kind.fields.iter().position(|f| f.name == word);
            let expected = if base(kind.base).entry(word).is_none() {
                None
            } else if let Some(index) = param {
                Some(BaseEntry::Param(index))
            } else {
                written.map(BaseEntry::Field)
            };
            assert_eq!(
                kind.base_entry(field),
                expected,
                "{} {word}",
                kind.decl.name
            );
        }
        if kind.base == BaseKind::Fastener {
            fasteners += 1;
            assert!(kind.base_entry(BaseField::Diameter).is_some());
            assert!(kind.base_entry(BaseField::Length).is_some());
        }
    }
    assert!(fasteners > 0, "the prelude's fasteners are resolved");
}
