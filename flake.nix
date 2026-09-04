{
  description = "yaac";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      ...
    }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system);

      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

      # Shared between the per-system package and the overlay.
      mkYaac =
        pkgs:
        let
          # Anki's build scripts read proto/ and ftl/ (with its translation
          # submodules) relative to the workspace root, which vendoring the crates
          # one by one drops. The tree is fetched once more for those directories;
          # keep the tag in step with Cargo.toml.
          ankiSrc = pkgs.fetchFromGitHub {
            owner = "ankitects";
            repo = "anki";
            tag = "26.08.1";
            fetchSubmodules = true;
            hash = "sha256-tRtxHQKmUFIBD/2WlxUx8ge1onB+gYFUo+NO/WPFqlU=";
          };
        in
        pkgs.rustPlatform.buildRustPackage {
          pname = "yaac";
          version = (fromTOML (builtins.readFile ./Cargo.toml)).package.version;
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
            allowBuiltinFetchGit = true;
          };
          # The vendored crates sit directly under the build root, so `../../proto`
          # from a crate directory lands here. The proto build also writes generated
          # Python and TypeScript under out/, and rslib includes the workspace's
          # .version file from the vendor directory's parent.
          postUnpack = ''
            ln -s ${ankiSrc}/proto "$NIX_BUILD_TOP/proto"
            ln -s ${ankiSrc}/ftl "$NIX_BUILD_TOP/ftl"
            mkdir -p "$NIX_BUILD_TOP/out"
          '';
          postPatch = ''
            cp ${ankiSrc}/.version "$cargoDepsCopy/.version"
          '';
          # Anki's rslib generates protobuf code at build time.
          nativeBuildInputs = [ pkgs.protobuf ];
          PROTOC = "${pkgs.protobuf}/bin/protoc";
          # The tests build throwaway collections through rslib and take a while;
          # `cargo test` runs them locally.
          doCheck = false;
          meta = {
            description = "Terminal Anki client built on Anki's own Rust backend";
            homepage = "https://github.com/kiliankoe/yaac";
            license = pkgs.lib.licenses.agpl3Plus;
            mainProgram = "yaac";
          };
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          yaac = mkYaac pkgs;
          default = mkYaac pkgs;
        }
      );

      overlays.default = final: _prev: { yaac = mkYaac final; };

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;

          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [
              "rust-src"
              "rust-analyzer"
            ];
          };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              rustToolchain
              # Anki's rslib generates protobuf code at build time.
              protobuf
            ];
          };
        }
      );
    };
}
