# Packed defaults — one shape across substrate + all of gen

> Status: in-flight. The substrate flag is wired; gen's CLI is the source of truth; the rename plumbing is the only remaining gap before the flag flips to default-on.

## What "packed defaults" means

Fleet-wide, every consumer should get the same default behavior from gen + substrate without having to know which subcommand to invoke or which flag to flip. The opt-out is an explicit deviation, never the path of least resistance.

## The unified surface — destination

```
# Generate everything from Cargo.toml + Cargo.lock + cargo metadata.
# One subcommand. Atomic. Replaces lock-build + lock-features + render.
gen build [path]

# substrate consumer — no flag needed once the rename plumbing lands:
substrate.mkCrate2nixProject {
  serviceName = "my-service";
  src = ./.;
  # default: useLockfileBuilder = true
}

# Operator opt-out for the legacy crate2nix-generated-Cargo.nix path:
substrate.mkCrate2nixProject {
  serviceName = "my-service";
  src = ./.;
  useGeneratedCargoNix = true;   # one-way escape hatch
}
```

## Current state (2026-05-26)

| Surface | Status |
|---|---|
| gen-cargo BuildSpec generation | ✓ ships, smoke-validated on tameshi-patent (77 crates) |
| gen-cargo lockfile duplicate-version handling | ✓ fixed (hashbrown 0.15.5 + 0.16.1 distinct) |
| gen-cargo features capture (renames + per-edge features) | ✓ ships |
| Cargo.build-spec.json schema | ✓ ships, ~75KB for 77-crate workspace |
| substrate `lockfile-builder.nix` orchestrator | ✓ 88 lines, reads spec, dispatches buildRustCrate |
| substrate `useLockfileBuilder` flag | ✓ wired into all three builders (Project / Tool / DockerImage) |
| Build through rustix (rename dep) | ✗ buildRustCrate doesn't honor the rename info our spec carries |
| substrate default flip | ⏸ blocked on rename plumbing |

## Path to packed default

1. **Port the rename plumbing** (~200 lines of Nix into lockfile-builder) so `buildRustCrate` receives properly-aliased dep derivations. Reference: crate2nix's `dependencyDerivations` + `filterEnabledDependencies` in `gen-nix/assets/crate2nix-internal-helpers.nix`.

2. **Consolidate gen CLI subcommands** behind `gen build`:
   - subcommand becomes atomic — runs lock-build + lock-features (deprecated) in one pass
   - sidecars become implementation detail
   - shikumi config drives output paths

3. **Flip substrate default**: `useLockfileBuilder ? true`. Rename today's opt-in flag to legacy opt-OUT `useGeneratedCargoNix ? false`.

4. **Sweep fleet**: run the substrate flake-check sweep across pleme-io's 508 cargo workspaces. Document trip rate (expected very low after fleet-coverage 100% in M1).

5. **Retire crate2nix from substrate inputs** (after stability window). At that point the regenerate step is gone fleet-wide.

## Cross-adapter consistency

The same `gen build` surface applies to every adapter:
- `gen build` in a Cargo.toml-rooted dir → BuildSpec via gen-cargo
- `gen build` in a package.json-rooted dir → BuildSpec via gen-npm
- `gen build` in a Gemfile-rooted dir → BuildSpec via gen-bundler

Each adapter produces the same Cargo.build-spec.json-shaped output (with adapter-appropriate Source variants — npm registry, RubyGems, etc.). The Nix orchestrator dispatches by file extension; the operator never thinks about which adapter is in play.
