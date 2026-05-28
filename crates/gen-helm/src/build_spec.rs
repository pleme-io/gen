//! Typed build spec for helm. Implements `gen_types::Spec`
//! via `#[derive(SpecShape)]`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, gen_macros::SpecShape)]
#[spec(
    args = "PackageArgs",
    quirk = "crate::quirks::HelmQuirk",
    args_field = "args",
    root_field = "root_package",
    members_field = "workspace_members",
    crates_field = "packages"
)]
pub struct BuildSpec {
    pub version: u32,
    pub packages: IndexMap<String, PackageSpec>,
    pub root_package: String,
    pub workspace_members: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageSpec {
    pub name: String,
    pub version: String,
    pub args: PackageArgs,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<crate::quirks::HelmQuirk>,
}

/// Pre-shaped chart-build args matching Helm's `Chart.yaml` (helm
/// v3 schema). Substrate's k8s consumer + FluxCD reconciler both
/// read these fields directly.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackageArgs {
    /// Chart name (`Chart.yaml: name`).
    pub name: Option<String>,
    /// Chart version (`Chart.yaml: version`) — semver.
    pub version: Option<String>,
    /// Optional appVersion (`Chart.yaml: appVersion`) — the version
    /// of the app the chart deploys, not the chart itself.
    #[serde(rename = "appVersion", skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    /// Chart type: `application` (default) or `library`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub chart_type: Option<String>,
    /// Description (`Chart.yaml: description`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `Chart.yaml: kubeVersion` — semver range.
    #[serde(rename = "kubeVersion", skip_serializing_if = "Option::is_none")]
    pub kube_version: Option<String>,
    /// Chart dependencies (`Chart.yaml: dependencies`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<ChartDependency>,
    /// Maintainers list.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub maintainers: Vec<ChartMaintainer>,
    /// Free-form topic keywords.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    /// Source URLs (e.g. github repo).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    /// `Chart.yaml: icon` — URL to the chart's icon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// `Chart.yaml: home` — chart's home URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// `Chart.yaml: deprecated` — marks chart deprecated; downstream
    /// consumers should refuse to render.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChartDependency {
    pub name: String,
    pub version: String,
    /// Repository URL (e.g. `https://charts.bitnami.com/bitnami`) or
    /// `oci://` reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Alias the dep is referenced by inside parent's
    /// `values.yaml` / templates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Condition expression — chart is included only if the named
    /// values entry is truthy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// Tag membership for grouped opt-in/out.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChartMaintainer {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}
