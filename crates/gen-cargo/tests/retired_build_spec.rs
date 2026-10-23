use std::path::Path;

use gen_cargo::build_spec::{FeatureSelection, SpecFile, generate_multi_target_and_write_with};

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, body).unwrap();
}

fn fixture(gitignore: &str) -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    write(
        root,
        "Cargo.toml",
        "[package]\nname = \"retired-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    );
    write(root, "src/lib.rs", "pub fn probe() {}\n");
    write(root, ".gitignore", gitignore);
    dir
}

#[test]
fn a_gitignored_build_spec_is_retired() {
    assert_eq!(
        SpecFile::for_root(fixture("/target\nCargo.build-spec.json\n").path()),
        SpecFile::Retired
    );
    assert_eq!(
        SpecFile::for_root(fixture("/target\n").path()),
        SpecFile::Write
    );
    assert_eq!(
        SpecFile::for_root(fixture("# Cargo.build-spec.json\n").path()),
        SpecFile::Write
    );
}

#[test]
fn a_retired_build_spec_is_not_written_and_the_delta_is() {
    let dir = fixture("Cargo.build-spec.json\n");
    let root = dir.path();
    let wrote = generate_multi_target_and_write_with(
        root,
        &FeatureSelection::default(),
        None,
        SpecFile::for_root(root),
    )
    .expect("gen build");
    assert_eq!(wrote, root.join("Cargo.gen.lock"));
    assert!(root.join("Cargo.gen.lock").exists());
    assert!(!root.join("Cargo.build-spec.json").exists());
}

#[test]
fn write_overrides_retirement() {
    let dir = fixture("Cargo.build-spec.json\n");
    let root = dir.path();
    generate_multi_target_and_write_with(root, &FeatureSelection::default(), None, SpecFile::Write)
        .expect("gen build");
    assert!(root.join("Cargo.build-spec.json").exists());
    assert!(root.join("Cargo.gen.lock").exists());
}

#[test]
fn an_explicit_out_path_is_always_written() {
    let dir = fixture("Cargo.build-spec.json\n");
    let root = dir.path();
    let wrote = generate_multi_target_and_write_with(
        root,
        &FeatureSelection::default(),
        Some(Path::new("Cargo.variant.build-spec.json")),
        SpecFile::Retired,
    )
    .expect("gen build");
    assert!(wrote.ends_with("Cargo.variant.build-spec.json"));
    assert!(wrote.exists());
}
