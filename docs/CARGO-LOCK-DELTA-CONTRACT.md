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
- `cargo_lock_sha256` — the **freshness tie, half 1 (resolution)**, computed
  over `Cargo.lock` (recomputable in Nix via `builtins.hashFile`)
- `manifest_sha256` — the **freshness tie, half 2 (declaration)**: a
  `workspace-root-relative path → sha256` map over the workspace root
  `Cargo.toml` plus every path-source package's `Cargo.toml`. Each entry is
  `builtins.hashFile "sha256"` of the same path

### Size
`Cargo.lock` (~56 KB) + gen-delta (~404 KB) ≈ **460 KB compact**, vs the current
full `Cargo.build-spec.json` at **1.56 MB** → **~3.4× smaller** committed weight,
same IFD-free purity, same per-crate fleet dedup.

## Invariants (CI-enforced)

- **D1 — No restatement.** The gen-delta MUST NOT carry any field in the
  "owned by `Cargo.lock`" list. A property test in gen-cargo asserts the delta
  has none of {source, registry sha256, dep closure, build_rust_crate_args,
  crate_renames, name/version}. If it does, the encoder regressed.
- **D2 — Freshness tie (TWO halves; both mandatory).** The tie's subject set is
  the pair `sha256(Cargo.lock)` ⊕ `{path → sha256}` over every workspace-local
  manifest. `cargo_lock_sha256` MUST equal `builtins.hashFile Cargo.lock`, and
  every `manifest_sha256` entry MUST equal `builtins.hashFile` of that path, at
  consume time. Mismatch ⇒ **hard eval error** (stale delta is a CI failure,
  never a runtime fetch — GEN TYPED-SPEC CONTRACT).

  **Why two halves — this was a real, measured hole, not a hypothetical.**
  Through delta schema v1 the tie hashed `Cargo.lock` alone. `Cargo.lock` pins
  *which packages, at which versions, from which sources* and pins **nothing**
  about the resolver facts the delta exists to carry. So a change that altered
  the resolved feature set without moving the lock left the tie **unchanged**
  and the staleness gate passed **green over a delta that no longer described
  the build**. Reproduced end-to-end: flipping `default = ["extra"]` →
  `default = []` leaves `Cargo.lock` byte-identical and changes
  `target_resolves[*].features`. The same shape covers `default-features`,
  `optional`, `edition`, `links`, `build`, `[lib]`, `[[bin]]`, `proc-macro` and
  `[package.metadata.pleme]` — every one declared in a workspace `Cargo.toml`.

  **The composition (why a recorded map is sound).** Half 1 already pins the
  *membership* of the local-manifest set — every workspace member and path dep
  has a `[[package]]` entry in `Cargo.lock`, so adding or removing one moves the
  lock and half 1 goes red before half 2 is consulted. Half 2 therefore only
  needs to cover *content* drift, which is exactly what the lock cannot express.

  **Deliberate exclusions** (an over-wide tie churns on unrelated edits and
  trains operators to regenerate blindly, which destroys the gate as surely as
  under-scoping): Rust sources; registry/git dependency manifests (pinned
  transitively by `checksum`/`#rev`); gen's own version and `FLEET_TARGETS`
  (carried by `schema_version`, and folding it in would invalidate every
  committed delta on every gen release). **Named residual risks:**
  `rust-toolchain.toml` / the cargo binary, `.cargo/config.toml`, and manifests
  outside the workspace root directory. Canonical statement, with the full
  reasoning per exclusion: `crates/gen-cargo/src/manifest_tie.rs`.

  **Migration.** Delta schema v1 → **v2**; spec schema v11 → **v12**. A v1 delta
  reads as the typed verdict `untied-manifests` — never as corruption and never
  rounded up to `fresh`. It gates strict `gen confirm` and is **tolerated by
  `gen confirm --if-present`** (the mode substrate's `gen-confirm` check uses),
  following the fleet's baseline-debt shape: the gate is adoptable the day it
  lands, and the debt shrinks monotonically because a pre-v12 spec classifies as
  `unhashed-spec` ⇒ `needs_regen()`, so the next `gen build --if-stale`
  regenerates a v2 delta. Measured migration surface at the time of the change:
  **413 committed `Cargo.gen.lock` across 406 repos, all at `schema_version: 1`.**

  **Consumer-side follow-up (NOT yet done — the tie is only as wide as its
  narrowest verifier).** `substrate/lib/build/rust/lockfile-builder.nix` still
  verifies only `cargo_lock_sha256`. Until it also walks `manifest_sha256`, the
  widened subject set is enforced by `gen confirm` (CI + the `gen-confirm`
  flake check), **not** at Nix eval.
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
