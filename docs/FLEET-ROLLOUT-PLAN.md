# Fleet rollout — substrate lockfile-builder default-on across pleme-io

> 399 spec sidecars sit untracked in pleme-io's cargo workspaces today. This plan walks the transition from "specs generated" → "specs committed + pushed + builds verified fleet-wide" → "crate2nix retired."

## Pre-flight (all must be green before Phase A)

- [ ] 10/10 canary `nix-build` matrix passes
  - caixa-sha2 ✓ caixa-encoding_rs ✓ hayai ✓ shikumi ✓ tameshi-patent ⏳ engawa-lisp ⏳ zoekt-mcp ⏳ mado ⏳ namimado ⏳ sui ⏳
- [ ] gen latest commit `e7cd82c` (build_script field) tagged + pushed
- [ ] substrate latest commit `006ed9b` (build threading) tagged + pushed
- [ ] Fleet sweep reports 0 invariant violations across 399 specs (verified earlier this session)

## Phase A — Re-sweep all 389 remaining repos for fresh spec generation

**Goal:** every repo's on-disk sidecar is at the latest gen-cargo schema (includes `build_script`, post-dev-dep-fix, post-v1-metadata-fix).

```sh
gen fleet-sweep ~/code/github/pleme-io --write --format yaml
```

**Verification:** 
- ok ≥ 399 (5 known cargo-state failures excluded)
- 0 invariant violations
- All 399 sidecars present at `<repo>/Cargo.build-spec.json`

**Confirmation point #1:** operator confirms sweep clean before Phase B starts.

**Rollback:** none needed — only writes to working trees, no git operations.

## Phase B — Commit + push in 8 waves of ~50 repos each

Wave-based to bound blast radius. Each wave: confirmation → execute → sample-verify → confirm next.

| Wave | Range | Repos |
|---|---|---:|
| 1 | a-c | ~52 |
| 2 | c-e | ~52 |
| 3 | e-h | ~52 |
| 4 | h-l | ~52 |
| 5 | l-p | ~52 |
| 6 | p-s | ~52 |
| 7 | s-v | ~52 |
| 8 | v-z | ~52 |

Per wave:
```sh
gen fleet-commit ~/code/github/pleme-io --push --rebase-first --format yaml \
  --include-prefix-range a-c   # (per-wave filter — to be added to fleet-commit if not present)
```

**Wave-success criteria:**
- All commits land cleanly (no GitCommitFailed / GitPushFailed in the typed report)
- Skipped-already-clean counted separately (idempotent — already-pushed canary repos report this)
- `git ls-remote` confirms HEAD matches local for each pushed repo

**Confirmation point #2-9:** operator confirms each wave clean before next wave starts.

**Rollback per wave:**
```sh
# Per affected repo:
git revert HEAD --no-edit && git push
# OR drop the sidecar entirely:
git rm Cargo.build-spec.json && git commit -m "revert spec rollout" && git push
```

## Phase C — Sample build verification per wave

After each commit wave: pick 5 random repos, run substrate-default-on build.

```sh
# 5 sample builds per wave (parallelizable):
for r in $(shuf -n 5 -e <wave-repos>); do
  nix-build /tmp/canary-test.nix --argstr src ~/code/github/pleme-io/$r --no-out-link
done
```

**Pass gate:** 5/5 builds produce `/nix/store/...` outputs.

**Failure gate:** any build failure → halt the wave sequence, triage the failure class, fix structurally in gen-cargo (per algorithmic discipline), refresh + re-sample.

## Phase D — Substrate flake bump fleet-wide

The lockfile-builder is default-on in substrate@006ed9b, but consumer repos pin substrate via flake input — they don't pick up the new default until they `nix flake update substrate`.

```sh
# Per substrate consumer repo:
nix flake update substrate
git add flake.lock
git commit -m "chore: bump substrate → lockfile-builder default-on" && git push
```

**Scope:** ~150-200 repos consume substrate (estimate from prior grep). Could be folded into Phase B commits where a repo gets both the sidecar AND the substrate bump in one commit. Operator decides.

**Confirmation point #10:** operator confirms Phase B done, then triggers Phase D.

## Phase E — 7-day stability window

After Phase D completes:
- Monitor every newly-merged PR across pleme-io for build failures attributable to gen
- Watch for new repo creations — repo-forge templates need `gen build .` added so day-1 repos ship a sidecar
- Track any GitHub issues filed against pleme-io/gen during the window
- Re-run fleet sweep weekly to catch lockfile drift

**No new architectural changes during the window.** Algorithmic discipline holds — patches are bugfixes for classes already covered, no new variants.

## Phase F — Retire crate2nix from substrate

After 7 stable days:
1. Remove `crate2nix` from substrate's flake inputs
2. Delete the `crate2nix-builders.nix` legacy path (or keep as historical reference)
3. Rename `mkCrate2nixProject` → `mkRustProject` (drop the historical name)
4. Document deprecation in substrate's CHANGELOG

**Rollback:** revert the substrate commit + repropagate via `nix flake update substrate` fleet-wide. Same mechanic as Phase D in reverse.

**Confirmation point #11:** operator confirms 7-day stability before retiring crate2nix.

## Algorithmic discipline (the principle held throughout)

Every failure that surfaces during rollout is treated as **a structural pattern to fix in gen-cargo**, never a per-repo workaround. The substrate Nix side is invariant — no consumer-side conditionals, no per-repo overrides.

Track-record this session:
- 5 structural classes surfaced + fixed:
  1. lockfile dup-version shadowing
  2. dep rename plumbing  
  3. lockfile-v1 metadata table
  4. dev-dep kind mis-classification
  5. non-root build.rs path threading

If a 6th class appears during rollout: stop the wave, add the typed field/variant + invariant test, re-sweep, resume.

## Confirmation gate summary

| Gate | Operator confirms | After |
|---|---|---|
| #1 | sweep clean | Phase A |
| #2-9 | each wave clean | each commit wave |
| #10 | Phase B done | before Phase D |
| #11 | 7-day stable | before Phase F |

11 explicit confirmation gates. Each is a place where rollout pauses unless the operator says go.
