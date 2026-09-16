# Workspace source for crane derivations. It holds the cargo-relevant files,
# which include Cargo.toml, Cargo.lock, *.rs and *.toml such as
# .cargo/config.toml and Trunk.toml. It also holds the .gantz assets and the
# .scm steel modules that crates include at compile time.
{ craneLib, lib }:
rec {
  root = ../.;
  fileset = lib.fileset.unions [
    (craneLib.fileset.commonCargoSources root)
    (lib.fileset.fileFilter (file: file.hasExt "gantz") root)
    (lib.fileset.fileFilter (file: file.hasExt "scm") root)
  ];
  src = lib.fileset.toSource { inherit root fileset; };
}
