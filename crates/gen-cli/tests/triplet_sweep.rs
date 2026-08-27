//! Triplet adoption sweep — verify every gen adapter Quirk
//! enum exposes the full typed-reflection triplet:
//!
//!   - TypedDispatcher (variant_kinds + variant_fields + variant_count)
//!   - Discriminant    (const fn discriminant() -> &'static str)
//!   - IsVariant       (const fn is_<variant>() -> bool per variant)
//!
//! Per theory/PATTERN-EXTRACTION.md Pattern 6 (sibling), this is
//! the canonical reflection shape for typed enums in pleme-io's
//! substrate. The sweep mechanically asserts:
//!
//! 1. The discriminant() method exists and returns the same
//!    string as variant_kinds()[i] for the i-th variant of a
//!    sample.
//! 2. The Discriminant-emitted method is callable.
//! 3. Every enum keeps its existing register_dispatcher!() entry
//!    (no regression from adding the derives).

use gen_ansible::quirks::AnsibleQuirk;
use gen_bundler::quirks::BundlerQuirk;
use gen_cargo::quirks::CrateQuirk;
use gen_gomod::quirks::GomodQuirk;
use gen_helm::quirks::HelmQuirk;
use gen_npm::quirks::NpmQuirk;
use gen_pip::quirks::PipQuirk;
use gen_platform::TypedDispatcherTrait;
use gen_poetry::quirks::PoetryQuirk;
use gen_swift::quirks::SwiftQuirk;

#[test]
fn cargo_quirk_triplet_coherent() {
    let q = CrateQuirk::ForceCfg { cfg: "x".into() };
    let kinds = CrateQuirk::variant_kinds();
    assert_eq!(q.discriminant(), "force-cfg");
    assert!(kinds.contains(&q.discriminant()));
    assert!(q.is_force_cfg());
    assert!(!q.is_fold_normal_into_build());
    assert!(!q.is_substitute_source());
}

#[test]
fn npm_quirk_triplet_coherent() {
    let q = NpmQuirk::SkipPostinstall;
    assert_eq!(q.discriminant(), "skip-postinstall");
    assert!(q.is_skip_postinstall());
    assert!(!q.is_pin_nodejs());
}

#[test]
fn bundler_quirk_triplet_coherent() {
    let q = BundlerQuirk::SkipNativeBuild;
    assert_eq!(q.discriminant(), "skip-native-build");
    assert!(q.is_skip_native_build());
    assert!(!q.is_pin_ruby());
}

#[test]
fn helm_quirk_triplet_coherent() {
    let q = HelmQuirk::SkipHook {
        hook: "pre-install".into(),
    };
    assert_eq!(q.discriminant(), "skip-hook");
    assert!(q.is_skip_hook());
    assert!(!q.is_override_value());
}

#[test]
fn pip_quirk_triplet_coherent() {
    let q = PipQuirk::SkipCheck;
    assert_eq!(q.discriminant(), "skip-check");
    assert!(q.is_skip_check());
    assert!(!q.is_pin_interpreter());
}

#[test]
fn poetry_quirk_triplet_coherent() {
    let q = PoetryQuirk::SkipCheck {
        package: "x".into(),
    };
    assert_eq!(q.discriminant(), "skip-check");
    assert!(q.is_skip_check());
    assert!(!q.is_override_attrs());
}

#[test]
fn gomod_quirk_triplet_coherent() {
    let q = GomodQuirk::CgoOff;
    assert_eq!(q.discriminant(), "cgo-off");
    assert!(q.is_cgo_off());
    assert!(!q.is_build_tag());
}

#[test]
fn ansible_quirk_triplet_coherent() {
    let q = AnsibleQuirk::DropDependency {
        collection: "broken.x".into(),
    };
    assert_eq!(q.discriminant(), "drop-dependency");
    assert!(q.is_drop_dependency());
    assert!(!q.is_build_ignore());
}

#[test]
fn swift_quirk_triplet_coherent() {
    let q = SwiftQuirk::Ldflag {
        flag: "-l-pthread".into(),
    };
    assert_eq!(q.discriminant(), "ldflag");
    assert!(q.is_ldflag());
    assert!(!q.is_pin_toolchain());
}

#[test]
fn every_adapter_discriminant_appears_in_variant_kinds() {
    macro_rules! check {
        ($ty:ty, $sample:expr) => {{
            let s = $sample;
            let kinds = <$ty as TypedDispatcherTrait>::variant_kinds();
            assert!(
                kinds.contains(&s.discriminant()),
                "{}: discriminant {} not in variant_kinds {:?}",
                stringify!($ty),
                s.discriminant(),
                kinds
            );
        }};
    }
    check!(CrateQuirk, CrateQuirk::ForceCfg { cfg: "x".into() });
    check!(NpmQuirk, NpmQuirk::SkipPostinstall);
    check!(BundlerQuirk, BundlerQuirk::SkipNativeBuild);
    check!(HelmQuirk, HelmQuirk::SkipHook { hook: "x".into() });
    check!(PipQuirk, PipQuirk::SkipCheck);
    check!(
        PoetryQuirk,
        PoetryQuirk::SkipCheck {
            package: "x".into()
        }
    );
    check!(GomodQuirk, GomodQuirk::CgoOff);
    check!(
        AnsibleQuirk,
        AnsibleQuirk::DropDependency {
            collection: "x".into()
        }
    );
    check!(SwiftQuirk, SwiftQuirk::Ldflag { flag: "x".into() });
}
