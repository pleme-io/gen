;;;; gen-gomod adapter — the authored Lisp spec (TYPED-SPEC + INTERPRETER
;;;; TRIPLET, item 2). Declares the adapter's verb surface + the M1
;;;; emission contract as Lisp data. The Rust border is
;;;; `gen-gomod/src/build_spec.rs`; the working interpreter is
;;;; `gen-gomod/src/interp.rs::apply`.

(defadapter gomod
  :ecosystem      "gomod"
  :manifest-files ("go.mod")
  :schema-version 2

  ;; Verb surface (gen_types::Adapter). Live in M1: build + confirm.
  :verbs
  ;; CORRECTED 2026-08-08: `:hermetic t` was FALSE, unconditionally stated for
  ;; a verb that is hermetic only on one of its two branches. `interp.rs:82-89`
  ;; probes for `vendor/modules.txt` and branches:
  ;;     vendored   -> -mod=<configured>  GOPROXY=off              (hermetic)
  ;;     otherwise  -> -mod=mod           GOPROXY=proxy.golang.org (NETWORK)
  ;; Claiming hermeticity for the branch that reaches the public module proxy
  ;; is backwards for anyone reasoning about sandboxing from this declaration —
  ;; and substrate carried the mirror-image of the same falsehood until
  ;; substrate@139248c.
  ((:name build   :status live        :hermetic :when-vendored
    :doc "go.mod + vendored tree -> Go.build-spec.json (per-package incremental)")
   (:name confirm :status live
    :doc "run the typed invariants over the encoded spec")
   (:name lock    :status unsupported :milestone "M-lock"
    :doc "go mod vendor/tidy — the resolver invocation (network)")
   (:name plan    :status unsupported :milestone "M2")
   (:name diff    :status unsupported :milestone "M2")
   (:name sbom    :status unsupported :milestone "M2")

   ;; The adoption census. Read-only BY CONSTRUCTION, not by convention:
   ;; `--dry-run` is the only mode the CLI defines, so there is no write path
   ;; to reach by accident. Declared here because "which modules can gen even
   ;; take?" is part of this adapter's surface, not a side tool.
   ;;
   ;; MEASURED 2026-08-08 over ~/code/github, 429 module roots:
   ;;   eligible              270
   ;;   bare-minor-directive  157   <- the escalation predicate; must reach 0
   ;;   no-directive            2   (one repo, both copies)
   ;;   above-fleet-toolchain   0   <- confirms the ONLY throwing arm ships
   ;;                                  with nothing to break
   ;; Re-measure, never infer: `gen adopt-go --dry-run --root <dir> --json`.
   (:name adopt   :status live :read-only t
    :doc "classify module roots against the fleet Go toolchain; measures, adopts nothing"
    :predicate "gen_gomod::directive — the SAME predicate substrate evaluates
                in Nix, bound to lib/build/go/directive-vectors.json so one byte
                edited there turns BOTH suites red"))

  ;; The `Go.gen.lock` producer, retired rather than deleted. The emitter is
  ;; compiled and tested; it has no consumer, so emitting would create a
  ;; freshness obligation nothing benefits from. See specs/go-delta.lisp.
  :delta (:spec "go-delta.lisp" :mode retired)

  ;; The resolver seam. `go list` IS the offline resolver (the
  ;; cargo-metadata analogue); it resolves build constraints, replace/
  ;; vendor rewriting, and the transitive closure — the encoder never
  ;; re-implements them. Abstracted behind the GoBuildEnv trait so tests
  ;; mock it.
  :resolver
  (:command "go list -deps -json -tags <tags> ./..."
   :env     (("GOFLAGS" . "-mod=vendor") ("GOPROXY" . "off")
             ("GOOS" . "<goos>") ("GOARCH" . "<goarch>") ("CGO_ENABLED" . "0"))
   :hermetic t
   :seam    "gen_gomod::interp::GoBuildEnv")

  ;; Typed quirk dispatcher — 5 arms, mirrored by
  ;; substrate/lib/build/gomod/quirk-apply.nix.
  :quirk-dispatcher "gen.gomod.gomod-quirk"

  ;; Content-address discipline.
  :hashes
  ((:field source_hash    :alg blake3 :over "sorted go_files ++ embed.files (length-prefixed)")
   (:field go_sum_sha256  :alg sha256 :over "go.sum content"
    :note "sha256 (not blake3) — Nix has builtins.hashFile \"sha256\"; empty ⇒ e3b0c4…")))
