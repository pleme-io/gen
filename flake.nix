{
  description = "gen — typed lockfile → Cargo.build-spec.json generator (Cargo / npm / Bundler / polyglot)";

  # substrate.rust.library dispatches over Cargo.gen.lock (the slim gen delta,
  # reconstructed to the full BuildSpec in pure Nix) — no crate2nix, no Cargo.nix.
  inputs.substrate.url = "github:pleme-io/substrate";

  outputs = { substrate, ... }:
    let
      base = substrate.rust.library {
        src = ./.;
        member = "gen-cli";
      };

      # ── GO_DIRECTIVE_VECTORS — the cross-repo table, published, not copied ──
      #
      # `crates/gen-gomod/tests/directive_vectors.rs` binds gen's `classify()`
      # to substrate's ONE committed vector table, the same file substrate's
      # `tests/directive-test.nix` reads. It deliberately REFUSES to skip when
      # the table is missing, and its only locator was a SIBLING GIT CHECKOUT
      # (`../../substrate/lib/build/go/directive-vectors.json`) — a layout every
      # dev machine has and no CI runner ever does.
      #
      # So the test passed locally and panicked in CI on every run from
      # 2026-08-08 (a046bcb) onward, taking `auto-release`'s Test gate down with
      # it and blocking every publish since. Measured, 2026-08-19 run 32288694298:
      #
      #     tried: Some("/home/runner/work/gen/substrate/lib/build/go/directive-vectors.json")
      #
      # The fix is to hand the test the file rather than to weaken the test or
      # to vendor a second copy of the table — a second copy is precisely the
      # defect the shared table exists to prevent. substrate is already a flake
      # input here, so its source is already on disk at the rev `flake.lock`
      # pins; this publishes that path into the devShell the release gate runs
      # in. One byte edited in substrate still turns both suites red, at the
      # granularity gen re-locks.
      #
      # This is also why the devShell is worth overriding rather than replacing:
      # substrate's `lib/util/test-env.nix` picks `.#default` when the consumer's
      # own shell can be entered (`tier=declared-devshell`) and falls back to
      # `github:pleme-io/substrate#release-gate` when it cannot
      # (`tier=substrate-fallback`, the weaker tier gen has been running at).
      # The env var only reaches the tests on the declared-devshell path, so the
      # substrate pin must stay new enough for `.#default` to be a plain
      # `pkgs.mkShell` — a pin older than substrate e232917 (2026-07-17) yields a
      # devenv-backed shell that CI cannot enter, and the gate silently drops
      # back to the fallback shell where this variable does not exist.
      directiveVectors = "${substrate}/lib/build/go/directive-vectors.json";
    in
    base
    // {
      devShells = builtins.mapAttrs (_system: shells:
        shells
        // {
          default = shells.default.overrideAttrs (_old: {
            GO_DIRECTIVE_VECTORS = directiveVectors;
          });
        }) base.devShells;
    };
}
