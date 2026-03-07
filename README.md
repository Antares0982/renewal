# renewal

Wraps some frequently used nixos-rebuild commands.

## Features

- `-u, --update` — update flakes (runs `git checkout <branch>`, `git pull` and `nix flake update --commit-lock-file`) before building.
- `-b, --build-only` — perform `nix build` only and do not run `nixos-rebuild switch`.
- `--remote target_host:remote_name` — use `remote_name` as the profile name and run a remote rebuild with:
	`nixos-rebuild switch --flake .#<remote_name> --target-host <target_host> --use-remote-sudo` (no `sudo` locally).
- `--branch <branch>` — git branch to checkout before building (default: `master`).
- `--no-pull` — skip the `git pull` step after checkout.

When no flags are given the tool will:

1. run `git checkout master` and `git pull` in the `NIX_DOT_FILES` directory (or `.` if unset);
2. run `nix build .#nixosConfigurations.$HOSTNAME.config.system.build.toplevel --show-trace`;
3. run `sudo nixos-rebuild switch --flake .#$HOSTNAME`.

## Usage

Build and run from the repository root:

```bash
renewal --update
renewal --build-only
renewal --remote myhost:remoteName
renewal --branch develop
renewal --no-pull
renewal --branch develop --no-pull
```

Or build a release binary and install it into your PATH.

## Notes

- The program reads the `NIX_DOT_FILES` environment variable to determine the repository directory. If unset, it defaults to the current working directory (`.`).
- This repository includes a `flake.nix` and `package.nix` for Nix builds; `package.nix` contains the `cargoHash` used by Nix.
- The tool exits with a non-zero status on command failures.
