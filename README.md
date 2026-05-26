# gen

Universal package-manager → build-system engine. One Rust engine
that supersedes the per-language `→ Nix` tool cohort (`crate2nix`,
`bundix`, `node2nix`, `poetry2nix`, `mix2nix`, …) by lifting the
80% of work that's identical across every language into a shared
typed core, leaving each ecosystem to contribute only its 20%
language-specific adapter.

See [`pleme-io/theory/GEN.md`](https://github.com/pleme-io/theory/blob/main/GEN.md)
for the full destination + architecture + milestone plan.

## Status

M0.0b — `gen-types` shipped. Typed IR (Manifest / Package /
Dependency / Feature / Constraint / Lockfile / Workspace /
BuildStep / Derivation). 58/58 tests pass.

## Crates

| crate | status | role |
|---|---|---|
| `gen-types` | M0.0b ✓ | typed IR every adapter + renderer + cache backend speaks |
| `gen-cargo` | pending | cargo manifest + lockfile parser → `Manifest` |
| `gen-cache-attic` | pending | attic-first `CacheBackend` impl |
| `gen-nix-incremental` | pending | per-crate `Derivation` emitter (crate2nix shape, typed AST) |
| `gen-cli` | pending | operator surface (`gen check / build / lock / publish`) |
| `gen-config` | pending | shikumi `TieredConfig` for the whole ecosystem |
