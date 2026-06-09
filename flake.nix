{
  description = "gen — typed lockfile → Cargo.build-spec.json generator (Cargo / npm / Bundler / polyglot)";

  nixConfig.allow-import-from-derivation = true;

  inputs = {
    nixpkgs.follows = "substrate/nixpkgs";
    crate2nix.url = "github:nix-community/crate2nix";
    flake-utils.url = "github:numtide/flake-utils";
    substrate = {
      url = "github:pleme-io/substrate";
    };
  };

  outputs = inputs: inputs.substrate.mkRustToolFlake {
    inherit inputs;
    src = ./.;
    member = "gen-cli";
  };
}
