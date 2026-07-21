# renewal

Wraps some frequently used nixos-rebuild commands.

Each host lives in its own flake under `hosts/<name>/`, where the single
`nixosConfigurations`/`darwinConfigurations` entry is named after the directory
(e.g. `hosts/rpi5#rpi5`). `NIX_DOT_FILES` points at the repo root, which is a
single git repository; all git operations run at the root while nix commands
target `./hosts/<name>`.

## Features

- `-u, --update` — update the target host's flake lock (runs `git checkout <branch>`, `git pull`, then `nix flake update --commit-lock-file` inside `hosts/<name>`) before building.
- `-b, --build-only` — perform `nix build` only and do not run `nixos-rebuild switch`.
- `--remote target_host:remote_name` — use `remote_name` as the profile name and run a remote rebuild with:
	`nixos-rebuild switch --flake ./hosts/<remote_name>#<remote_name> --target-host <target_host> --sudo` (no `sudo` locally).
- `--branch <branch>` — git branch to checkout before building (default: `master`).
- `--no-pull` — skip the `git pull` step after checkout.

When no flags are given the tool will:

1. run `git checkout master` and `git pull` in the `NIX_DOT_FILES` directory (or `.` if unset);
2. run `nix build ./hosts/$HOSTNAME#nixosConfigurations.$HOSTNAME.config.system.build.toplevel --show-trace`;
3. run `sudo nixos-rebuild switch --flake ./hosts/$HOSTNAME#$HOSTNAME`.

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
