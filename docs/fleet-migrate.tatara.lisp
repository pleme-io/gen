;; fleet-migrate.tatara.lisp — DELTA-ONLY spec migration plan.
;;
;; Compiled by `gen fleet-migrate <this-file>` into a typed
;; FleetMigrationPlan (#[derive(TataraDomain)]). Each eligible repo under
;; :workspace-root is converted to the delta-only doctrine: regenerate the
;; spec, commit the slim Cargo.gen.lock, retire (git rm + gitignore) the
;; full Cargo.build-spec.json. substrate's lockfile-builder reconstructs
;; the spec from Cargo.lock + the delta (delta > build-spec > IFD).
;;
;; The engine classifies each repo with a typed outcome and is safe to
;; re-run (idempotent): already-migrated repos report already-delta-only,
;; repos with their own uncommitted work are skipped untouched, and repos
;; whose `gen build` re-resolves Cargo.lock (an unpatched git self-ref)
;; are skipped + restored rather than committed.
;;
;; Booleans are Scheme-style #t / #f.

(fleet-migration-plan
  :workspace-root "~/code/github/pleme-io"

  ;; Empty :repos → discover every eligible repo (Cargo.toml + .git +
  ;; a committed build-spec or delta) under the workspace root.

  ;; Excluded repos:
  ;;   gen          — has its own gen-spec-verify CI; stays delta-only via
  ;;                  `gen build --commit` (commit_spec_if_drifted).
  ;;   ishou        — unpatched transitive git self-ref; `gen build`
  ;;                  re-resolves Cargo.lock. Needs a [patch] first (the
  ;;                  engine skips it as lock-mutated regardless).
  ;;   tatara-lisp  — authoring surface this plan depends on; leave alone.
  :exclude ("gen" "ishou" "tatara-lisp")

  ;; Push with fetch + ff-only + SHA verification after each commit.
  :push #t

  ;; Commit as the ambient git author (operator-driven rollout). The
  ;; autonomous CI path commits as gen-spec-bot separately.
  :bot-identity #f)
