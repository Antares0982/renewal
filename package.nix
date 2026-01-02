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
  cargoHash = "sha256-T55ece63jUDZQBS70CcjcjHYFrKJqxkVMEahYBVv/Co=";
}
