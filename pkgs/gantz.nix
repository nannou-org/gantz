{
  gantz-unwrapped,
  lib,
  makeWrapper,
  stdenv,
  symlinkJoin,
}:
symlinkJoin {
  name = "gantz";
  buildInputs = [ makeWrapper ];
  paths = [ gantz-unwrapped ];
  postBuild = lib.optionalString stdenv.hostPlatform.isLinux ''
    wrapProgram $out/bin/gantz \
      --set LD_LIBRARY_PATH "${gantz-unwrapped.LD_LIBRARY_PATH}"
  '';
}
