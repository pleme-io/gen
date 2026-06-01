# The `Cargo.lock` ⇄ gen-delta Contract

**Status:** canonical. Extends the org-level **GEN TYPED-SPEC CONTRACT**
(`theory/GEN-TYPED-SPEC-CONTRACT.md`). Owned by `gen-cargo` (producer) +
`substrate/lib/build/rust/lockfile-builder.nix` (consumer).

## Intent (the relationship we commit to maintaining)

`Cargo.lock` is **upstream's resolution pin** — the authoritative, standard,
already-committed record of *which crate versions* and *their source hashes*.
We **use it for exactly what it is good for** and never restate it.

cargo's *resolver* produces facts `Cargo.lock` does **not** record — resolved
features per target, cfg/target dep gating, dep-kind split, proc-macro/tree
placement, edition, build-script/links/lib targets. Those are real, needed for
**per-crate** building (the cross-repo Nix-store dedup that is the whole reason
gen + lockfile-builder exist at 500+ repos), and they **cannot be derived from
`Cargo.lock`**. We capture *only those* in **our own lock — the gen-delta**
(`Cargo.gen.lock`, JSON).

**The standing commitment:** `Cargo.lock` stays the source of truth for
everything it expresses; the gen-delta carries *only* the resolver delta on top
of it; the two are kept in lockstep by `cargo_lock_sha256`; and we never let the
gen-delta drift back into restating lock data. Less is more — the delta is the
*smallest* artifact that preserves per-crate fleet-dedup with zero IFD.

## The split (field-exact, 2026-06-01 compute)

### Owned by `Cargo.lock` — derived in PURE Nix, never committed in the delta
`builtins.fromTOML(Cargo.lock)` (+ `fromTOML(Cargo.toml)`) reconstructs all of:
- crate **name**, **version**
- **source** kind + url discriminator (`registry+…` / `git+url#rev` / path)
- registry **sha256** (the `checksum` field — cargo-metadata doesn't even
  surface these; the lock is authoritative)
- git **rev** (embedded in `git+url#rev`)
- registry URL + `name_with_ext` (synthesized from name+version)
- the **dependency closure** (the flat which-depends-on-which edge set)
- `root_crate`, `workspace_members`, member `relative_path`
  (from `[workspace].members` + lock versions)
- `crate_renames`, `build_rust_crate_args`, the runtime/build dep restatements —
  all pure restatements of the above

### Owned by the gen-delta (`Cargo.gen.lock`) — the must-commit resolver facts
Not in `Cargo.lock`, not derivable, required at **eval** time for per-crate emit:
- **Per fleet target** (×6 `FLEET_TARGETS`): per-crate **resolved features**
  (the lock has *zero* feature data, and they differ per target) + per-edge
  `{package_key, kind (normal|build|dev), target (cfg gate), tree (target|host),
  features, optional, uses_default_features}`
- **Per-crate scalars** (target-independent): `edition`, `proc_macro`,
  `build_script`, `links`, `lib_target {name,path}`, `binaries [{name,path}]`,
  `quirks`
- **git-source NAR sha256** (the lock has the rev; the fixed-output hash is gen's
  gix+NAR prefetch)
- `flake_metadata.module_trio` (from `[package.metadata.pleme]`)
- `cargo_lock_sha256` — the **freshness tie**, computed over `Cargo.lock`
  (recomputable in Nix via `builtins.hashFile`)

### Size
`Cargo.lock` (~56 KB) + gen-delta (~404 KB) ≈ **460 KB compact**, vs the current
full `Cargo.build-spec.json` at **1.56 MB** → **~3.4× smaller** committed weight,
same IFD-free purity, same per-crate fleet dedup.

## Invariants (CI-enforced)

- **D1 — No restatement.** The gen-delta MUST NOT carry any field in the
  "owned by `Cargo.lock`" list. A property test in gen-cargo asserts the delta
  has none of {source, registry sha256, dep closure, build_rust_crate_args,
  crate_renames, name/version}. If it does, the encoder regressed.
- **D2 — Freshness tie.** `cargo_lock_sha256` in the gen-delta MUST equal
  `builtins.hashFile Cargo.lock` at consume time. Mismatch ⇒ **hard eval error**
  (stale delta is a CI failure, never a runtime fetch — GEN TYPED-SPEC CONTRACT).
- **D3 — Pure reconstruction.** lockfile-builder MUST reconstruct every
  lock-owned field via `fromTOML`, never `fromJSON` of restated lock data, and
  never IFD / `cargo metadata` / network at eval.
- **D4 — Delta is non-empty by necessity.** It cannot be reduced to zero without
  forfeiting per-crate dedup (crane-style single-derivation builds do that — see
  "Why not crane"). The floor is the resolver delta, not nothing.

## Maintenance model (automatic, never hand-touched)

- `gen lock` (network, the one non-hermetic operator verb) refreshes `Cargo.lock`.
- `gen build` (run in **CI**, never at nix-eval) regenerates `Cargo.gen.lock`
  from the fresh `Cargo.lock` + cargo-metadata, and CI auto-commits it (gated by
  `cargo_lock_sha256`). One reusable workflow, fleet-wide, zero `flake.nix` edits.
- nix-eval reads `Cargo.lock` (pure) + the committed `Cargo.gen.lock` (pure) →
  per-crate `buildRustCrate` → built once on rio, pulled by all. No IFD, ever.

## Why not crane (just `Cargo.lock`, no delta)

crane needs only `Cargo.lock` because it lets **cargo resolve features at build
time inside one workspace derivation** — so there is **no per-crate derivation
and no cross-repo dedup**: a crate compiled in repo A is not shared with repo B.
For pleme-io's 500+ repos with heavy shared deps, that dedup is load-bearing
(one `anstream-1.0.0` drv fleet-wide, not recompiled per repo). gen pushes
resolution to eval time precisely to keep it — which is *why* it must commit the
resolver delta crane lets cargo compute later. The delta is the price of dedup,
and it's a small one.

## Generalization (rust is the POC)

The same contract shape repeats per ecosystem: `<eco>.lock` (upstream pin,
pure-Nix-derived) + `<eco>.gen-delta` (resolver facts the lock can't express),
consumed by an `<eco>-lockfile-builder`. npm (`package-lock.json`), python
(`uv.lock`), go (`go.sum`) all carry checksums + closure; each gets a slim delta
for whatever its resolver adds. The four-state lock-lifecycle FSM is shared.
