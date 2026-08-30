//! `DispatcherCatalog` end-to-end — register a synthetic typed
//! enum via `register_dispatcher!`, iterate the catalog, look up
//! by label, verify reflection round-trip.

use gen_platform::{TypedDispatcher, catalog, register_dispatcher};
use serde::{Deserialize, Serialize};

/// Local synthetic enum mirroring a real migration shape. The
/// catalog primitive is exercised against this; gen-cargo's
/// CrateQuirk and the other 8 production enums register
/// independently when their crates pull gen-platform.
#[derive(Clone, Debug, Serialize, Deserialize, TypedDispatcher)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[allow(dead_code)]
enum DemoMigration {
    AddColumn {
        table: String,
        column: String,
        ty: String,
    },
    DropColumn {
        table: String,
        column: String,
    },
    Backfill {
        table: String,
        sql: String,
    },
}

register_dispatcher!("gen-platform.demo.migration", DemoMigration);

#[test]
fn catalog_contains_registered_demo() {
    let all = catalog::registered();
    assert!(
        all.iter().any(|e| e.label == "gen-platform.demo.migration"),
        "registered() must include the demo entry; got: {:?}",
        all.iter().map(|e| e.label).collect::<Vec<_>>()
    );
}

#[test]
fn by_label_resolves_registered_entry() {
    let entry = catalog::by_label("gen-platform.demo.migration").expect("entry must exist");
    assert_eq!(entry.label, "gen-platform.demo.migration");
    assert_eq!((entry.variant_count)(), 3);
}

#[test]
fn variant_kinds_round_trip_through_catalog() {
    let entry = catalog::by_label("gen-platform.demo.migration").unwrap();
    let kinds = (entry.variant_kinds)();
    assert_eq!(kinds, vec!["add-column", "drop-column", "backfill"]);
}

#[test]
fn variant_fields_round_trip_through_catalog() {
    let entry = catalog::by_label("gen-platform.demo.migration").unwrap();
    let fields = (entry.variant_fields)();
    assert_eq!(
        fields,
        vec![
            ("add-column", vec!["table", "column", "ty"]),
            ("drop-column", vec!["table", "column"]),
            ("backfill", vec!["table", "sql"]),
        ]
    );
}

#[test]
fn unknown_label_returns_none() {
    assert!(catalog::by_label("does.not.exist").is_none());
}
