{
  description = "gen — typed lockfile → Cargo.build-spec.json generator (Cargo / npm / Bundler / polyglot)";

  # substrate.rust.library dispatches over Cargo.gen.lock (the slim gen delta,
  # reconstructed to the full BuildSpec in pure Nix) — no crate2nix, no Cargo.nix.
  inputs.substrate.url = "github:pleme-io/substrate";

  outputs = { substrate, ... }: substrate.rust.library {
    src = ./.;
    member = "gen-cli";
  };
}
