{
  description = "dumb";

  inputs = {
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.follows = "rust-overlay/nixpkgs";
    naersk.url = "github:nix-community/naersk";
  };

  outputs = inputs: with inputs; flake-utils.lib.eachDefaultSystem (system:
    let
      pkgs = import nixpkgs { overlays = [ (import rust-overlay) ]; inherit system; };
      naerskLib = pkgs.callPackage naersk {
        cargo = rust-toolchain;
        rustc = rust-toolchain;
      };
      buildInputs = with pkgs; [
      ];
      nativeBuildInputs = with pkgs; [
      ];
      rust-toolchain = pkgs.rust-bin.stable.latest.default.override {
        extensions = [ "rust-src" "rustfmt" "rust-docs" "clippy" ];
      };
      LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath buildInputs}";
      # Allow cargo to pull from private git repositories via local SSH key.
      CARGO_NET_GIT_FETCH_WITH_CLI = "true";
    in rec {
      packages = {
        dumb = naerskLib.buildPackage {
          name = "dumb";
          src = ./.;
          inherit buildInputs LD_LIBRARY_PATH CARGO_NET_GIT_FETCH_WITH_CLI;
          nativeBuildInputs = nativeBuildInputs;
        };
        default = packages.dumb;
      };
      devShells.default = pkgs.mkShell {
        inherit buildInputs LD_LIBRARY_PATH CARGO_NET_GIT_FETCH_WITH_CLI;
        nativeBuildInputs = nativeBuildInputs ++ [ rust-toolchain pkgs.rust-analyzer ];
        RUST_BACKTRACE = 1;
      };
    }
  );
}
