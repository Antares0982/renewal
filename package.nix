{
  pkgs,
  rustPlatform,
  lib,
  ...
}:
rustPlatform.buildRustPackage rec {
  pname = "renewal";
  version = "1.0.0";
  src = builtins.path {
    name = pname;
    path = ./.;
  };
  cargoHash = "sha256-fhZB0dlBdoLc1aY+R2R4BYr7pMNAeji9C7Rf9srAMs0=";
  nativeBuildInputs = [ pkgs.installShellFiles ];
  nativeCheckInputs = [ pkgs.git ];
  postInstall = lib.optionalString (pkgs.stdenv.buildPlatform.canExecute pkgs.stdenv.hostPlatform) ''
    installShellCompletion --cmd renewal --zsh <($out/bin/renewal --completions zsh)
  '';
}
