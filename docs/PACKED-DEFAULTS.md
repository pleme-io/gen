# Packed defaults — one shape across substrate + all of gen

> Status: **shipped + fleet-validated** as of 2026-05-26. The substrate flag flipped to default-on; the lockfile-builder consumes a typed Cargo.build-spec.json that gen-cargo synthesizes; 399/404 cargo workspaces in pleme-io generate clean specs on first contact.

## The architectural invariant

**Rust (gen) owns ALL semantics. Nix (substrate) is pure dispatch.**

| Layer | Rust (gen-cargo) | Nix (substrate lockfile-builder) |
|---|---|---|
| Parsing | Cargo.toml + Cargo.lock (v1 + v2/v3) + cargo metadata | one `builtins.fromJSON` |
| Resolution | features, cfg() via `--filter-platform`, renames, dep splits | none |
| Source URL + sha256 synthesis | yes — `fetchurl` args pre-shaped | none |
| Build-shape transformation | runtime/build dep split + crate_renames table pre-shaped | none |
| Derivation construction | n/a | per-crate `buildRustCrate` call |
| Dep graph walk | spec is the graph | memoized recursion via `buildByKey` |

Every field in Cargo.build-spec.json arrives in its final consumer-ready shape. The Nix file is invariant against future cargo quirks — new cargo behavior lands in Rust.

## The operator surface

```sh
# Canonical: generate every sidecar the substrate consumer needs.
gen build [path]

# Multi-repo: typed fleet sweep with structural failure categorization.
gen fleet-sweep <root> [--write]

# Lower-level (rarely needed):
gen lock-build [path]   # just the BuildSpec
gen lock-features [path] # just the features sidecar
```

```nix
# Substrate consumer — default-on, no flag.
substrate.mkCrate2nixProject {
  serviceName = "my-service";
  src = ./.;
}

# Legacy opt-out:
substrate.mkCrate2nixProject {
  serviceName = "my-service";
  src = ./.;
  useLockfileBuilder = false;   # falls back to crate2nix generated Cargo.nix
}
```

## Fleet-sweep results (2026-05-26)

```yaml
total: 777     # immediate sub-dirs of pleme-io
ok: 399        # cargo workspaces with successful BuildSpec generation
skipped: 373   # no Cargo.toml or no Cargo.lock (non-cargo repos)
failed: 5      # upstream cargo state problems

total_spec_bytes: 105_787_651   # ~100MB across the fleet
elapsed_ms: 458_352             # 7.6 minutes for the full sweep
```

Failure categorization (structural, not per-repo):

| Category | Count | Repos |
|---|---|---|
| GitFetchFailed | 3 | arachne-plugins, train-forge, weights-forge |
| VersionResolutionFailed | 1 | caixa-clap |
| WorkspaceMemberInvalid | 1 | lilitu-web |

All 5 failures are upstream cargo-state problems — cargo metadata itself can't proceed. Not gen bugs.

## Algorithmic discipline (the principle)

When a failure surfaces during fleet sweep, the response is **structural extraction in gen-cargo**, never a one-off Nix-side conditional:

- **v1 lockfile metadata table** (surfaced by caixa-encoding_rs) → added `metadata: IndexMap` field to `raw::CargoLock` + `lookup_metadata_checksum` in `convert.rs`. Now every v1 lockfile in the fleet (and the future) flows through correctly.
- **Lockfile duplicate-version shadowing** (surfaced by tameshi-patent's hashbrown 0.15.5 + 0.16.1) → keyed ID index by `(name, version)` instead of `name` alone. Structural fix, not per-version handling.
- **cfg() target evaluation in Nix** (caught during the rename plumbing port) → moved to Rust via `cargo metadata --filter-platform=<host>`. Nix never evaluates cfg() — that's the Rust side's job.
- **buildRustCrate rename plumbing** (surfaced by rustix's `libc_errno` alias) → ported crateRenames synthesis to Rust; emit pre-shaped attrset in the spec; Nix passes through verbatim.

The contract is one schema; deviations are spec bugs, never consumer bugs.

## Rollout state

| Phase | Status |
|---|---|
| gen ecosystem shipped (13 crates, 158 tests) | ✓ |
| substrate orchestrator (121 lines pure dispatch) | ✓ |
| substrate default-on flag | ✓ |
| Fleet sweep (399/404 generate clean specs) | ✓ |
| Spec sidecars written to working trees | ✓ |
| Per-repo commit + push wave | ⏸ (high blast radius — needs operator decision on approach) |
| Substrate flake-check fleet sweep | ⏸ (requires per-repo flake update to pick up substrate@350bc0f+) |
| crate2nix removal from substrate inputs | ⏸ (post-stability window) |
