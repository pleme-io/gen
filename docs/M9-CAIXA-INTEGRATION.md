# M9 — Caixa integration: (defcaixa …) auto-emits gen.lisp

> Status: typed-surface stub landed in this commit (`gen-caixa-bridge` crate). Authoring-side Lisp form lands when tatara-lisp's defform machinery accepts the typed bridge.

## Destination

Every `(defcaixa …)` block in the fleet auto-emits a `gen.lisp` alongside `caixa.lisp` describing the package-manager shape — operators never hand-author `gen render` invocations. The bridge is the typed seam: caixa knows about gen, not the other way around.

## Typed surface (already in place — `gen-caixa-bridge`)

```rust
pub struct CaixaToGen {
    /// The caixa kind drives the adapter selection.
    pub caixa_kind: CaixaKind,
    /// Path to the caixa source root (the dir holding caixa.lisp).
    pub source_root: PathBuf,
    /// Operator-provided override for the adapter selection.
    pub force_adapter: Option<String>,
    /// Render mode (per-crate / per-tree) from caixa.lisp's :build slot.
    pub render_mode: RenderMode,
}

impl CaixaToGen {
    /// Probe the source root + select the right adapter; return a
    /// typed AdapterRoute the caixa renderer can act on.
    pub fn route(&self) -> Result<AdapterRoute, CaixaBridgeError> { ... }
}
```

## Adapter routing per caixa kind

| Caixa kind | Default adapter | Notes |
|---|---|---|
| `Biblioteca` | cargo | Rust library → `Cargo.toml` |
| `Binario` | cargo | Rust binary → `Cargo.toml` |
| `Servico` | cargo | Rust service → `Cargo.toml`; nix-render via per-crate |
| `Supervisor` | cargo | Rust supervisor → `Cargo.toml` |
| `Aplicacao` | polyglot | Can be Rust + Yew (wasm) + Ruby tooling → all three |

## Authoring shape (proposed)

```lisp
(defcaixa my-thing
  :kind :biblioteca
  :gen (:adapter :cargo
        :render-mode :per-crate
        :nix-output "Cargo.nix"))
```

The `:gen` slot is optional; defaults derive from `:kind` per the table above.

## Generated artifacts

For each caixa, the gen-caixa-bridge emits one or more:

- `Cargo.nix` (for Rust caixas) — produced by `gen render . --output-path Cargo.nix`
- `package-lock.nix` (for Aplicacao caixas with package.json) — gen-npm + gen-nix
- `Gemfile.nix` (for Aplicacao caixas with Gemfile) — gen-bundler + gen-nix
- `gen.lisp` — meta-spec describing which adapters were dispatched

The `gen.lisp` is committed alongside `caixa.lisp` so the substrate can audit "which manifest synthesized what".

## Acceptance gate

- One `Aplicacao` caixa in pleme-io declares `:gen` slot.
- Re-rendering the caixa via `feira render` produces `gen.lisp` + matching artifacts.
- The artifacts build via `nix-build` without operator intervention.
- Skill update: `caixa-author` SKILL.md gets a `:gen` section.

## Why this lands last

M9 needs M0-M8 to be solid (every adapter ships, every renderer ships, byte-parity polish done). Landing it earlier would tie caixa to a moving target; landing it last means caixa picks up a frozen, fleet-validated typed surface.
