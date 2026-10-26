# Workspace source for crane derivations. It holds the cargo-relevant files,
# which include Cargo.toml, Cargo.lock, *.rs and *.toml such as
# .cargo/config.toml and Trunk.toml. It also holds the files that crates
# include at compile time: the .gantz assets, the .scm steel modules, and the
# .svg logos and .ttf fonts of the UI.
{ craneLib, lib }:
rec {
  root = ../.;
  fileset = lib.fileset.unions [
    (craneLib.fileset.commonCargoSources root)
    (lib.fileset.fileFilter (file: file.hasExt "gantz") root)
    (lib.fileset.fileFilter (file: file.hasExt "scm") root)
    (lib.fileset.fileFilter (file: file.hasExt "svg") root)
    (lib.fileset.fileFilter (file: file.hasExt "ttf") root)
  ];
  src = lib.fileset.toSource { inherit root fileset; };
}
