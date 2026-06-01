#!/usr/bin/env bash
# commit-gen-spec.sh — one-time migration: adopt the committed-spec IFD-free
# path in one repo. Deterministic, safe-by-default. Usage:
#   commit-gen-spec.sh <repo-dir> [--push]
# Exits 0 and prints a STATUS=<word> line for the caller to aggregate.
#
# NOTE: this is one-shot migration glue. The durable form is a `gen` subcommand
# (`gen build --commit`) — tracked as the Rust-native follow-up. Run this only
# AFTER gen-cargo emits the SLIM delta (schema 9): committing today's FULL spec
# (0.2–6.5 MB each) would permanently bloat git history fleet-wide.
set -euo pipefail
dir="${1:?usage: commit-gen-spec.sh <repo-dir> [--push]}"
push="${2:-}"
cd "$dir"

say() { echo "STATUS=$1 ${2:-}"; }

[ -f Cargo.toml ] || { say skipped-not-gen "no Cargo.toml"; exit 0; }
[ -z "$(git status --porcelain)" ] || { say skipped-dirty "tree not clean"; exit 0; }

# 1. un-ignore the spec if the repo gitignores it (the fleet convention we flip)
if git check-ignore -q Cargo.build-spec.json 2>/dev/null; then
  # remove the exact line from .gitignore (any leading path forms)
  if [ -f .gitignore ]; then
    grep -vxE '/?Cargo\.build-spec\.json' .gitignore > .gitignore.tmp || true
    mv .gitignore.tmp .gitignore
  fi
fi

# 2. generate the spec (network for cargo metadata — the one-time cost)
if ! gen build . >/tmp/gen-build.log 2>&1; then
  git checkout -- . 2>/dev/null || true
  say failed-build "$(tail -1 /tmp/gen-build.log)"
  exit 0
fi
[ -f Cargo.build-spec.json ] || { git checkout -- . 2>/dev/null || true; say failed-build "no spec produced"; exit 0; }

# 3. stage ONLY the expected paths: the spec, the .gitignore edit, the
#    deprecated crate-hashes.json removal (gen supersedes it), a refreshed lock.
git add Cargo.build-spec.json
[ -f .gitignore ] && git add .gitignore
git rev-parse --verify HEAD:crate-hashes.json >/dev/null 2>&1 && [ ! -f crate-hashes.json ] && git rm --quiet --cached crate-hashes.json 2>/dev/null || true
[ -f Cargo.lock ] && git add Cargo.lock

# guard: nothing unexpected got staged
unexpected="$(git diff --cached --name-only | grep -vxE 'Cargo\.build-spec\.json|\.gitignore|crate-hashes\.json|Cargo\.lock' || true)"
[ -z "$unexpected" ] || { git reset --quiet; git checkout -- . 2>/dev/null || true; say skipped-dirty "unexpected staged: $unexpected"; exit 0; }
[ -n "$(git diff --cached --name-only)" ] || { say skipped-no-change "nothing to commit"; exit 0; }

git commit --quiet -m 'chore(gen): commit Cargo.build-spec.json + un-ignore (IFD-free builds)' \
  -m 'Adopt the committed-spec path: nix-eval reads the spec (pure cp), no eval-time gen build / cargo metadata / network. Un-ignores the spec; drops the deprecated crate-hashes.json (superseded by the spec source hashes).'

kb=$(( $(wc -c < Cargo.build-spec.json) / 1024 ))
if [ "$push" = "--push" ]; then
  if git push --quiet origin HEAD:main 2>/tmp/gen-push.log; then say pushed "${kb}KB"; else say failed-push "$(tail -1 /tmp/gen-push.log)"; fi
else
  say committed-local "${kb}KB (not pushed)"
fi
