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
  ((:name build   :status live        :hermetic t
    :doc "go.mod + vendored tree -> Go.build-spec.json (per-package incremental)")
   (:name confirm :status live
    :doc "run the typed invariants over the encoded spec")
   (:name lock    :status unsupported :milestone "M-lock"
    :doc "go mod vendor/tidy — the resolver invocation (network)")
   (:name plan    :status unsupported :milestone "M2")
   (:name diff    :status unsupported :milestone "M2")
   (:name sbom    :status unsupported :milestone "M2"))

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
