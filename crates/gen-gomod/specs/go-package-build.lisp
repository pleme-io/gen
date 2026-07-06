;;;; The gomod per-package build algorithm — authored Lisp spec (item 2
;;;; of the triplet). Both engines (this authored spec + the Rust
;;;; interpreter in `gen-gomod/src/interp.rs`) drive the SAME phase list;
;;;; drift is impossible. A node = one Go package compiled for one
;;;; target tuple; the load-bearing technique is rustc-per-crate, in Go.

(defgo-package-build-algorithm gomod-incremental
  :renderer incremental
  :dep-mode vendored           ; M1: -mod=vendor, GOPROXY=off, zero network
  :node     (import-path . target-tuple)   ; key = "<import-path>#<goos>-<goarch>[+tags]"

  ;; Encoder phases — walked once at emit time (interp::apply). Each
  ;; phase is mockable via GoBuildEnv; a phase that cannot complete
  ;; returns a typed SpecError naming it (never a silent wrong answer).
  :phases
  ((:name read-go-mod    :input  "go.mod"
    :produces module-spec :via "GoMod::parse")
   (:name go-list        :env    GoBuildEnv
    :produces "concatenated go list -json objects")
   (:name parse-go-list  :produces "Vec<GoListPackage>" :via "golist::parse_stream")
   (:name reject-cgo     :guard  Go-I12
    :fail-closed t :doc "cgo sources on a non-std node ⇒ typed rejection (deferred to M-cgo)")
   (:name reject-asm     :guard  Go-I12
    :fail-closed t :doc "asm sources on a non-std node ⇒ typed rejection (std asm lives in the opaque std-tree; deferred to M-asm)")
   (:name relative-path  :guard  Go-I3  :produces "PackageSource::Vendored{relative_path}")
   (:name read-source    :env    GoBuildEnv :produces "go_files ++ embed.files bytes")
   (:name source-hash    :guard  Go-I8  :produces "blake3 content address")
   (:name resolve-imports :guard Go-I1  :produces "edge -> node key (through ImportMap)")
   (:name embed          :guard  Go-I9  :produces "EmbedSpec{patterns,files}")
   (:name tree           :guard  Go-I2  :produces "BuildTree (M1: every node Target)")
   (:name roots          :produces "root_package (smallest main) + workspace_members")
   (:name compact-resolves :produces "GoCompactTargetResolves (single tuple, M-multitarget-ready)")
   (:name go-sum-tie     :guard  Go-I7  :produces "go_sum_sha256"))

  ;; Package kinds (M1). Cgo/Tool are deferred — structurally
  ;; unrepresentable in the M1 enum, so Go-I12 is enforced at encode via
  ;; the two fail-closed phases reject-cgo + reject-asm.
  :kinds (std module main)

  ;; Invariants — asserted on BOTH sides (encoder property test +
  ;; substrate interpreter defensive synthesis). See invariants.rs.
  :invariants
  ((:id Go-I1  :rule "every import edge resolves to a node in packages")
   (:id Go-I2  :rule "each node's tree is Target (workload/std); Host reserved for Tool/Cgo")
   (:id Go-I3  :rule "Vendored relative_path is a real subdir (no .., not absolute)")
   (:id Go-I6  :rule "build constraints resolved to a concrete go_files list BY THE ENCODER")
   (:id Go-I7  :rule "go_sum_sha256 = sha256(go.sum); empty ⇒ e3b0c4…")
   (:id Go-I8  :rule "non-std node carries a blake3 source_hash (incremental cache key)")
   (:id Go-I9  :rule "embed patterns imply embed files (-embedcfg)")
   (:id Go-I10 :rule "std ⟺ Std source; non-std never Std")
   (:id Go-I11 :rule "every workspace_member is a Main node")
   (:id Go-I12 :rule "no cgo/asm node in the M1 subgraph — rejected at encode"))

  ;; Substrate interpreter (the Nix consumer of this spec).
  :interpreter "substrate/lib/build/go/package-builder.nix"
  :std-tree    "one derivation per (goVersion, goos, goarch, tags)"
  :compile     "go tool compile -p <import-path> -importcfg <cfg> [-embedcfg] -o pkg.a <go_files>"
  :link        "go tool link -importcfg <cfg> -o bin/<name> pkg.a   ; kind=main only")
