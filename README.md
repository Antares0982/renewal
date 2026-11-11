# renewal

Wraps some frequently used nixos-rebuild commands.

Each host lives in its own flake under `hosts/<name>/`, where the single
`nixosConfigurations`/`darwinConfigurations` entry is named after the directory
(e.g. `hosts/rpi5#rpi5`). `NIX_DOT_FILES` points at the repo root, which is a
single git repository; all git operations run at the root while nix commands
target `./hosts/<name>`.

## Features

- Automatically use `nom build` when `nom --version` succeeds, letting nix-output-monitor handle the JSON log pipeline and preserve build failures; otherwise use `nix build`.
- `-u, --update` — update the target host's flake lock (runs `git checkout <branch>`, `git pull`, then `nix flake update --commit-lock-file` inside `hosts/<name>`) before building.
- `-b, --build-only` — perform `nix build` only and do not run `nixos-rebuild switch`.
- `--remote target_host:remote_name` — use `remote_name` as the profile name and run a remote rebuild with:
	`nixos-rebuild switch --flake ./hosts/<remote_name>#<remote_name> --target-host <target_host> --sudo` (no `sudo` locally).
  When no colon is given, both names use the same value: `--remote gz` is equivalent to `--remote gz:gz`.
- `--branch <branch>` — git branch to checkout before building (default: `master`).
- `--no-pull` — skip the `git pull` step after checkout.

When no flags are given the tool will:

1. run `git checkout master` and `git pull` in the `NIX_DOT_FILES` directory (or `.` if unset);
2. run `nix build ./hosts/$HOSTNAME#nixosConfigurations.$HOSTNAME.config.system.build.toplevel --show-trace`;
3. run `nixos-rebuild switch --flake ./hosts/$HOSTNAME#$HOSTNAME --sudo`.

On Linux, evaluation and building run as the current user; `nixos-rebuild`
uses sudo only for privileged deployment steps. This requires a version of
`nixos-rebuild` that supports `--sudo` (such as the NixOS 25.11 implementations).
Sudo rules must allow the deployment commands invoked by `nixos-rebuild`;
an allowance for only `sudo nixos-rebuild` is insufficient.

## Usage

Build and run from the repository root:

```bash
renewal --update
renewal --build-only
renewal --remote myhost:remoteName
renewal --remote gz
renewal --branch develop
renewal --no-pull
renewal --branch develop --no-pull
```

Or build a release binary and install it into your PATH.

## Zsh completion

The Nix package installs zsh completion automatically. Enable completion in
your shell with `autoload -Uz compinit && compinit` if it is not already enabled.

For a manual installation, generate the script:

```zsh
mkdir -p ~/.zsh/completions
renewal --completions zsh > ~/.zsh/completions/_renewal
```

Add these lines to `~/.zshrc`, placing the `fpath` line before any existing
`compinit` call:

```zsh
fpath=(~/.zsh/completions $fpath)
autoload -Uz compinit && compinit
```

Completion covers options and argument descriptions for the current platform;
it does not query branches or hosts. Generating the script does not access
`NIX_DOT_FILES` or run Git/Nix commands.

## Notes

- The program reads the `NIX_DOT_FILES` environment variable to determine the repository directory. If unset, it defaults to the current working directory (`.`).
- An empty or invalid `NIX_DOT_FILES` is an error. The directory must be a Git working tree, including with `--build-only`; only switching requires a clean working tree.
- Errors include the operation, command, working directory, and underlying cause. Failed quiet commands include their captured output. A branch without an upstream skips pulling with a warning.
- Darwin host detection silently skips flakes with missing or empty `darwinConfigurations`; evaluation failures still produce warnings. Use `--hostname <name>` to select a host explicitly and skip scanning.
- If tagging or pushing fails after switching, the error reports that the switch already succeeded. A failed push leaves the local tag in place and prints a retry command.
- This repository includes a `flake.nix` and `package.nix` for Nix builds; `package.nix` contains the `cargoHash` used by Nix.
- The tool exits with a non-zero status on command failures.
