# M8 — Retire crate2nix in substrate

> Status: planning. Code is in place (gen 100% fleet-cargo parse + nix-build smoke passed). What remains is the substrate-side cutover.

## Destination

`substrate/lib/rust-*-flake.nix` builders generate `Cargo.nix` via `gen render` instead of `crate2nix generate`. Operators don't have to install crate2nix anymore; the fleet builds against one Rust binary.

## Substrate touch-points

Every flake builder that invokes crate2nix today. Grep target:

```sh
grep -rln "crate2nix" ~/code/github/pleme-io/substrate/lib/
```

Expected matches (from prior fleet review):

- `substrate/lib/rust-tool-release-flake.nix` — single-CLI Rust tool
- `substrate/lib/rust-workspace-release-flake.nix` — multi-crate workspace
- `substrate/lib/rust-library.nix` — crates.io library
- `substrate/lib/rust-action-release-flake.nix` — GitHub Actions

Each invokes `crate2nix generate` in a `regenerate` flake-app or pre-build hook.

## Cutover steps (per builder)

1. Add `gen.url = "github:pleme-io/gen"` to the builder's expected inputs.
2. Replace the `regenerate` app body:
   - **Before**: `${crate2nix}/bin/crate2nix generate --output Cargo.nix`
   - **After**: `${gen}/bin/gen render . --output-path Cargo.nix`
3. Run `nix run .#regenerate` on a canary repo (suggest `pleme-io/caixa-bitflags` — smoke-validated in M0.5).
4. Compare store paths between crate2nix-generated and gen-generated `Cargo.nix` builds:
   - Functional parity: both `nix-build`s succeed → ✓
   - Byte-identical store paths: M0.5b polish (currently differ; documented in M0.5 commit)
5. Land per builder; substrate releases follow the standard auto-release flow.

## Per-repo migration

Each consumer rebuilds via `nix flake update substrate && nix run .#regenerate`. Old `Cargo.nix` is overwritten in place; no schema change visible to operators.

## Compatibility window

- Phase A (M0.5b → M1): both crate2nix + gen ship; substrate defaults to crate2nix; opt-in via `pkgs.useGen = true` in the builder args.
- Phase B (M2): substrate defaults to gen; crate2nix kept as opt-out (`pkgs.useGen = false`).
- Phase C (M3+): crate2nix removed from substrate inputs entirely; consumers who need it pin their own.

## Rollback plan

If a consumer trips on a parse gap: file the failing Cargo.toml minimum-repro against `pleme-io/gen`, set `pkgs.useGen = false` for that repo's flake, continue using crate2nix until the gap is patched. Fleet sweep methodology validated in M1 — 100% pass rate on 508 repos means trip rate is empirically low.

## Acceptance gate

- Substrate's CI sweep of `nix flake check` on all builders is green.
- Three canary repos (caixa-bitflags / hanabi / libkrun-builder) build via `gen render` + `nix-build` successfully.
- No regressions reported in fleet rebuild for one week.

## Open follow-ups (do NOT block M8)

- M0.5b — byte-identical store-path parity (requires resolvedDefaultFeatures emission + edition propagation).
- `gen render` --strict to fail on any condition crate2nix would have stripped silently.
- Substrate-side `gen` overlay so consumers can override the binary version per-repo.
