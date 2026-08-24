# Workspace source for crane derivations: cargo-relevant files (Cargo.toml,
# Cargo.lock, *.rs, *.toml - including .cargo/config.toml and Trunk.toml) plus
# the .gantz assets that gantz_base and gantz_plyphon include at compile time
# and the .scm steel modules that gantz_core includes at compile time.
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
