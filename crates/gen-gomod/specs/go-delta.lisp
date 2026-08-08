;; go-delta.lisp — the declared policy for gen-gomod's `Go.gen.lock` producer.
;;
;; RETIRED, and this declaration is the switch. The emitter in
;; `src/gen_delta.rs` is complete, compiled and tested; it simply has no
;; consumer, so emitting the artifact would create a freshness obligation
;; nothing benefits from.
;;
;; Measured 2026-08-08: `write_gen_delta` had ZERO call sites (gen-cargo's
;; sibling has one), and 0 `Go.gen.lock` exist fleet-wide against a 425-file
;; `Cargo.gen.lock` control.
;;
;; MODULARIZE, DON'T DELETE: reviving is `:mode "active"` here — not an
;; archaeology exercise. `tests/spec_parity.rs` asserts this file and
;; `DeltaPolicy::RETIRED` agree, so the declaration cannot drift from the code.
(defgodelta go-delta
  :mode "retired"
  :reason "no-consumer"
  :emitter "src/gen_delta.rs"
  :artifact "Go.gen.lock"
  :revive "set :mode \"active\"; the emitter is already compiled and tested")
