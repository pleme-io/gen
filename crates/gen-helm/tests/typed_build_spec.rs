use gen_helm::build_spec::{
    BuildSpec, ChartDependency, ChartMaintainer, PackageArgs, PackageSpec, SCHEMA_VERSION,
};
use gen_helm::invariants::HelmInvariants;
use gen_helm::quirks::{HelmQuirk, HelmQuirks};
use gen_types::{Adapter, Invariants, QuirkRegistry, Spec};
use indexmap::IndexMap;

#[test]
fn package_args_serializes_to_chart_yaml_shape() {
    let args = PackageArgs {
        name: Some("argocd".into()),
        version: Some("5.51.4".into()),
        app_version: Some("v2.10.0".into()),
        chart_type: Some("application".into()),
        description: Some("ArgoCD".into()),
        kube_version: Some(">=1.22.0".into()),
        dependencies: vec![ChartDependency {
            name: "redis".into(),
            version: "17.0.0".into(),
            repository: Some("https://charts.bitnami.com/bitnami".into()),
            alias: Some("cache".into()),
            condition: Some("redis.enabled".into()),
            tags: vec!["backend".into()],
        }],
        maintainers: vec![ChartMaintainer {
            name: "drzzln".into(),
            email: Some("a@b".into()),
            url: None,
        }],
        keywords: vec!["gitops".into()],
        sources: vec!["https://github.com/argoproj/argo-cd".into()],
        ..Default::default()
    };
    let v: serde_json::Value = serde_json::to_value(&args).unwrap();
    // Critical Chart.yaml shape:
    assert_eq!(v.get("name"), Some(&serde_json::json!("argocd")));
    assert_eq!(v.get("version"), Some(&serde_json::json!("5.51.4")));
    assert_eq!(v.get("appVersion"), Some(&serde_json::json!("v2.10.0")));
    assert_eq!(v.get("type"), Some(&serde_json::json!("application")));
    assert_eq!(v.get("kubeVersion"), Some(&serde_json::json!(">=1.22.0")));
    let deps = v.get("dependencies").unwrap().as_array().unwrap();
    assert_eq!(deps[0]["name"], serde_json::json!("redis"));
    assert_eq!(
        deps[0]["repository"],
        serde_json::json!("https://charts.bitnami.com/bitnami")
    );
    assert_eq!(deps[0]["alias"], serde_json::json!("cache"));
    assert_eq!(deps[0]["condition"], serde_json::json!("redis.enabled"));
}

#[test]
fn quirk_variants_round_trip() {
    for q in [
        HelmQuirk::OverrideValue {
            path: "image.tag".into(),
            value: serde_json::json!("v1.2.3"),
        },
        HelmQuirk::SkipHook {
            hook: "pre-install".into(),
        },
        HelmQuirk::PatchManifest {
            resource_kind: "Deployment".into(),
            name: "app".into(),
            jsonpath: "$.spec.template.spec.serviceAccountName".into(),
            value: serde_json::json!("custom-sa"),
        },
        HelmQuirk::ForceImageTag {
            container: "main".into(),
            tag: "v1.0.0".into(),
        },
    ] {
        let v: serde_json::Value = serde_json::to_value(&q).unwrap();
        assert!(v.get("kind").is_some());
        let back: HelmQuirk = serde_json::from_value(v).unwrap();
        assert_eq!(q, back);
    }
}

#[test]
fn build_spec_implements_spec_trait() {
    let mut packages = IndexMap::new();
    packages.insert(
        "argocd-5.51.4".into(),
        PackageSpec {
            name: "argocd".into(),
            version: "5.51.4".into(),
            args: PackageArgs {
                name: Some("argocd".into()),
                version: Some("5.51.4".into()),
                ..Default::default()
            },
            quirks: vec![],
        },
    );
    let spec = BuildSpec {
        version: SCHEMA_VERSION,
        packages,
        root_package: "argocd-5.51.4".into(),
        workspace_members: vec!["argocd-5.51.4".into()],
    };
    assert_eq!(spec.schema_version(), SCHEMA_VERSION);
    assert_eq!(spec.root_key(), "argocd-5.51.4");
    let violations = <HelmInvariants as Invariants>::check(&spec);
    assert!(
        violations.is_empty(),
        "minimal spec violated: {violations:?}"
    );
}

#[test]
fn registry_is_empty_default() {
    assert!(<HelmQuirks as QuirkRegistry>::registered_names().is_empty());
}

#[test]
fn adapter_quirks_registry_envelope() {
    let a = gen_helm::adapter::HelmAdapter;
    assert!(a.quirks_registry().is_empty());
}
