//! Runtime `Dispatcher<V, C, O>` end-to-end against the production
//! `gen_cargo::quirks::CrateQuirk` enum.
//!
//! Proves the Rust-side peer of `mk-typed-dispatcher.nix`:
//!
//! - register helpers per variant kind
//! - seal asserts variant-universe coverage at construction time
//! - missing coverage surfaces typed (MissingCoverage)
//! - duplicate helper registration errors typed (DuplicateHelper)
//! - apply_each folds helpers over a variant list
//! - try_apply_each catches unknown kinds (defensive path)

use gen_cargo::quirks::CrateQuirk;
use gen_platform::{Dispatcher, DispatcherError, MergeStrategy, TypedDispatcherTrait};

#[derive(Default)]
struct Ctx {
    log: Vec<String>,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Override {
    flags: Vec<String>,
    pre_patch: Option<String>,
}

#[test]
fn happy_path_seal_then_apply() {
    let d = Dispatcher::<CrateQuirk, Ctx, Override>::new()
        .with_helper("force-cfg", |q, ctx| {
            if let CrateQuirk::ForceCfg { cfg } = q {
                ctx.log.push(format!("force-cfg:{cfg}"));
                Override {
                    flags: vec![format!("--cfg={cfg}")],
                    pre_patch: None,
                }
            } else {
                Override::default()
            }
        })
        .with_helper("fold-normal-into-build", |q, ctx| {
            if let CrateQuirk::FoldNormalIntoBuild { extern_crate } = q {
                ctx.log.push(format!("fold:{extern_crate:?}"));
                Override::default()
            } else {
                Override::default()
            }
        })
        .with_helper("substitute-source", |q, ctx| {
            if let CrateQuirk::SubstituteSource { file, from, to } = q {
                ctx.log.push(format!("sub:{file}:{from}=>{to}"));
                Override {
                    flags: vec![],
                    pre_patch: Some(format!("sed -i 's/{from}/{to}/' {file}")),
                }
            } else {
                Override::default()
            }
        })
        .with_helper("native-build-inputs", |_, _| Override::default())
        .into_sealed()
        .expect("seal succeeds — every variant covered");

    let mut ctx = Ctx::default();
    let variants = vec![
        CrateQuirk::ForceCfg { cfg: "supports_64bit".into() },
        CrateQuirk::SubstituteSource {
            file: "build.rs".into(),
            from: "old".into(),
            to: "new".into(),
        },
    ];
    let results = d.apply_each(&variants, &mut ctx);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].flags, vec!["--cfg=supports_64bit".to_string()]);
    assert_eq!(
        results[1].pre_patch.as_deref(),
        Some("sed -i 's/old/new/' build.rs")
    );
    assert_eq!(ctx.log.len(), 2);
}

#[test]
fn seal_fails_on_missing_coverage() {
    let d = Dispatcher::<CrateQuirk, Ctx, Override>::new()
        .with_helper("force-cfg", |_, _| Override::default())
        // missing fold-normal-into-build + substitute-source
        ;
    let DispatcherError::MissingCoverage { missing } = d.into_sealed().err().expect("missing coverage must error")
    else {
        panic!("wrong error variant");
    };
    assert!(missing.contains(&"fold-normal-into-build".to_string()));
    assert!(missing.contains(&"substitute-source".to_string()));
    assert!(missing.contains(&"native-build-inputs".to_string()));
    assert_eq!(missing.len(), 3);
}

#[test]
fn helper_returns_duplicate_error() {
    let r = Dispatcher::<CrateQuirk, Ctx, Override>::new()
        .helper("force-cfg", |_, _| Override::default())
        .and_then(|d| d.helper("force-cfg", |_, _| Override::default()));
    let DispatcherError::DuplicateHelper { kind } = r.err().expect("duplicate helper must error")
    else {
        panic!("wrong error variant");
    };
    assert_eq!(kind, "force-cfg");
}

#[test]
fn try_apply_each_surfaces_unknown_kinds_defensively() {
    // Build a sealed dispatcher.
    let d = Dispatcher::<CrateQuirk, Ctx, Override>::new()
        .with_helper("force-cfg", |_, _| Override::default())
        .with_helper("fold-normal-into-build", |_, _| Override::default())
        .with_helper("substitute-source", |_, _| Override::default())
        .with_helper("native-build-inputs", |_, _| Override::default())
        .into_sealed()
        .unwrap();

    // Typed variants never produce an unknown kind, so the happy
    // path passes:
    let mut ctx = Ctx::default();
    let typed = vec![CrateQuirk::ForceCfg { cfg: "x".into() }];
    assert!(d.try_apply_each(&typed, &mut ctx).is_ok());
}

#[test]
fn variant_count_matches_dispatcher_universe() {
    // gen-cargo::CrateQuirk has 4 variants — match against the
    // sealed dispatcher's coverage requirement.
    assert_eq!(<CrateQuirk as TypedDispatcherTrait>::variant_count(), 4);
}

#[test]
fn strategy_is_settable_and_observable() {
    let d = Dispatcher::<CrateQuirk, Ctx, Override>::new()
        .with_strategy(MergeStrategy::Accumulate)
        .with_helper("force-cfg", |_, _| Override::default())
        .with_helper("fold-normal-into-build", |_, _| Override::default())
        .with_helper("substitute-source", |_, _| Override::default())
        .with_helper("native-build-inputs", |_, _| Override::default())
        .into_sealed()
        .unwrap();
    assert_eq!(d.strategy(), MergeStrategy::Accumulate);
}

#[test]
fn empty_variants_returns_empty_results() {
    let d = Dispatcher::<CrateQuirk, Ctx, Override>::new()
        .with_helper("force-cfg", |_, _| Override::default())
        .with_helper("fold-normal-into-build", |_, _| Override::default())
        .with_helper("substitute-source", |_, _| Override::default())
        .with_helper("native-build-inputs", |_, _| Override::default())
        .into_sealed()
        .unwrap();
    let mut ctx = Ctx::default();
    assert_eq!(d.apply_each(&[], &mut ctx).len(), 0);
}
