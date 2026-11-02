use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, ensure};
use clap::{CommandFactory, Parser};

const BLUE: &str = "\x1b[34m";
const ORANGE: &str = "\x1b[38;5;208m";
const NORMAL: &str = "\x1b[0m";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Print shell completion script and exit
    #[arg(long, value_parser = ["zsh"])]
    completions: Option<String>,

    /// Update flakes before building
    #[arg(short = 'u', long = "update")]
    update: bool,

    /// Build without switching
    #[arg(short = 'b', long = "build-only")]
    build_only: bool,

    /// Remote as `target_host:remote_name` or `name`
    #[cfg(target_os = "linux")]
    #[arg(long = "remote")]
    remote: Option<Remote>,

    /// Git branch to checkout (default: master)
    #[arg(long = "branch", default_value = "master")]
    branch: String,

    /// Skip `git pull` after checkout
    #[arg(long = "no-pull")]
    no_pull: bool,

    /// Host under hosts/; auto-detected if omitted
    #[cfg(target_os = "macos")]
    #[arg(long = "hostname")]
    hostname: Option<String>,

    /// Skip tagging and pushing after switching
    #[arg(long = "no-tag")]
    no_tag: bool,
}

#[cfg(target_os = "linux")]
mod remote {
    use std::str::FromStr;

    #[derive(Debug, Clone)]
    pub struct Remote {
        pub target_host: String,
        pub remote_name: String,
    }

    impl FromStr for Remote {
        type Err = String;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let (target_host, remote_name) = s.split_once(':').unwrap_or((s, s));
            if target_host.is_empty() || remote_name.is_empty() {
                return Err("remote must be in format target_host:remote_name or name".into());
            }
            Ok(Remote {
                target_host: target_host.to_string(),
                remote_name: remote_name.to_string(),
            })
        }
    }

    #[cfg(test)]
    #[test]
    fn parses_remote() {
        use clap::Parser;

        for (input, host, name) in [
            ("gz", "gz", "gz"),
            ("gz:gz", "gz", "gz"),
            ("user@host:profile", "user@host", "profile"),
        ] {
            let cli = crate::Cli::try_parse_from(["renewal", "--remote", input]).unwrap();
            let remote = cli.remote.unwrap();
            assert_eq!(remote.target_host, host);
            assert_eq!(remote.remote_name, name);
        }
        for input in ["", ":", "gz:", ":gz"] {
            assert!(crate::Cli::try_parse_from(["renewal", "--remote", input]).is_err());
        }
    }
}

#[cfg(target_os = "linux")]
use remote::Remote;

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.completions.is_some() {
        let mut script = Vec::new();
        clap_complete::generate(
            clap_complete::shells::Zsh,
            &mut Cli::command(),
            "renewal",
            &mut script,
        );
        std::io::stdout()
            .write_all(&script)
            .context("failed to write zsh completion script to stdout")?;
        return Ok(());
    }

    let workdir = repository_dir()?;
    let workdir = workdir.as_path();
    check_repository(workdir, !cli.build_only)?;

    do_git_checkout_pull(workdir, &cli.branch, cli.no_pull)?;

    #[cfg(target_os = "linux")]
    run_nixos(workdir, &cli)?;

    #[cfg(target_os = "macos")]
    run_darwin(workdir, &cli)?;

    Ok(())
}

fn repository_dir() -> anyhow::Result<PathBuf> {
    let (path, source) = match env::var_os("NIX_DOT_FILES") {
        Some(value) => {
            ensure!(
                !value.is_empty(),
                "NIX_DOT_FILES is empty; set it to your Nix configuration repository or unset it to use the current directory"
            );
            (PathBuf::from(value), "NIX_DOT_FILES")
        }
        None => (
            PathBuf::from("."),
            "current directory (NIX_DOT_FILES is unset)",
        ),
    };
    let resolved = path.canonicalize().with_context(|| {
        format!("cannot access repository directory {path:?} from {source}; check that the path exists and is accessible")
    })?;
    let metadata = std::fs::metadata(&resolved).with_context(|| {
        format!("cannot inspect repository directory {resolved:?} from {source}")
    })?;
    ensure!(
        metadata.is_dir(),
        "repository path {resolved:?} from {source} is not a directory"
    );
    Ok(resolved)
}

#[cfg(target_os = "linux")]
fn run_nixos(workdir: &Path, cli: &Cli) -> anyhow::Result<()> {
    let effective_name = if let Some(ref r) = cli.remote {
        r.remote_name.clone()
    } else {
        hostname::get()
            .context("failed to read the local hostname for selecting hosts/<name>")?
            .into_string()
            .map_err(|name| anyhow::anyhow!("local hostname {name:?} is not valid UTF-8"))?
    };
    check_host(workdir, &effective_name)?;

    if cli.update {
        do_flake_update(&host_dir(workdir, &effective_name))?;
    }

    println!("{}Host: {}{}", BLUE, effective_name, NORMAL);
    println!("{}Building...{}", BLUE, NORMAL);
    do_nix_build(workdir, &effective_name, "nixosConfigurations")?;

    if cli.build_only {
        return Ok(());
    }

    println!("{}Switching...{}", BLUE, NORMAL);
    do_nixos_rebuild_switch(workdir, cli.remote.as_ref(), &effective_name)?;

    if !cli.no_tag {
        tag_and_push(workdir, &effective_name)
            .context("system switch succeeded, but tagging or pushing the configuration failed")?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn run_darwin(workdir: &Path, cli: &Cli) -> anyhow::Result<()> {
    let hostname = match &cli.hostname {
        Some(h) => h.clone(),
        None => resolve_darwin_hostname(workdir)?,
    };
    check_host(workdir, &hostname)?;

    if cli.update {
        do_flake_update(&host_dir(workdir, &hostname))?;
    }

    println!("{}Building...{}", BLUE, NORMAL);
    do_nix_build(workdir, &hostname, "darwinConfigurations")?;

    if cli.build_only {
        return Ok(());
    }

    println!("{}Switching...{}", BLUE, NORMAL);
    do_darwin_rebuild_switch(workdir, &hostname)?;

    if !cli.no_tag {
        tag_and_push(workdir, &hostname)
            .context("system switch succeeded, but tagging or pushing the configuration failed")?;
    }

    Ok(())
}

/// Find the Darwin host flake.
#[cfg(target_os = "macos")]
fn resolve_darwin_hostname(dir: &Path) -> anyhow::Result<String> {
    let hosts_dir = dir.join("hosts");
    let mut found: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(&hosts_dir)
        .with_context(|| format!("failed to scan host directory {hosts_dir:?}"))?
    {
        let entry = entry.with_context(|| format!("failed to read an entry in {hosts_dir:?}"))?;
        if !entry
            .file_type()
            .with_context(|| format!("failed to read file type for {:?}", entry.path()))?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name().into_string().map_err(|name| {
            anyhow::anyhow!("host directory name {name:?} in {hosts_dir:?} is not valid UTF-8")
        })?;
        let flake = entry.path().join("flake.nix");
        if !flake
            .try_exists()
            .with_context(|| format!("failed to access {flake:?}"))?
        {
            continue;
        }

        let mut cmd = Command::new("nix");
        cmd.arg("eval")
            .arg(host_flake_ref(&name, "darwinConfigurations"))
            .arg("--apply")
            .arg("x: builtins.attrNames x")
            .arg("--json");
        let output = match capture_in_dir(&mut cmd, dir) {
            Ok(output) => output,
            Err(error) => {
                eprintln!("Warning: could not inspect Darwin host {name:?}; skipping: {error:#}");
                continue;
            }
        };

        let names: Vec<String> = serde_json::from_slice(&output.stdout).with_context(|| {
            format!(
                "invalid darwinConfigurations JSON from {} in {dir:?}: {}",
                command_text(&cmd),
                String::from_utf8_lossy(&output.stdout)
            )
        })?;
        if !names.is_empty() {
            found.push(name);
        }
    }

    match found.len() {
        0 => Err(anyhow::anyhow!(
            "no darwinConfigurations found under {hosts_dir:?}; inspect any warnings above or specify --hostname <name>"
        )),
        1 => {
            println!("{}Auto-detected darwin host: {}{}", BLUE, found[0], NORMAL);
            Ok(found[0].clone())
        }
        _ => Err(anyhow::anyhow!(
            "multiple Darwin hosts found under {hosts_dir:?}: {}. Specify --hostname <name>.",
            found.join(", ")
        )),
    }
}

fn command_text(cmd: &Command) -> String {
    std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_in_dir(cmd: &mut Command, dir: &Path) -> anyhow::Result<()> {
    let command = command_text(cmd);
    let status = cmd.current_dir(dir).status()
        .with_context(|| format!("could not run {command} in {dir:?}; check the executable, PATH, and directory permissions"))?;
    ensure!(
        status.success(),
        "command {command} failed in {dir:?} ({status}); see command output above"
    );
    Ok(())
}

fn capture_in_dir(cmd: &mut Command, dir: &Path) -> anyhow::Result<Output> {
    let command = command_text(cmd);
    let output = cmd.current_dir(dir).output()
        .with_context(|| format!("could not run {command} in {dir:?}; check the executable, PATH, and directory permissions"))?;
    ensure!(
        output.status.success(),
        "command {command} failed in {dir:?} ({})\nstderr: {}\nstdout: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim(),
        String::from_utf8_lossy(&output.stdout).trim(),
    );
    Ok(output)
}

fn do_git_checkout_pull(dir: &Path, branch: &str, no_pull: bool) -> anyhow::Result<()> {
    let output = capture_in_dir(
        Command::new("git").args(["rev-parse", "--abbrev-ref", "HEAD"]),
        dir,
    )
    .context("failed to determine the current Git branch")?;
    let prev_branch = String::from_utf8_lossy(&output.stdout);
    let prev_branch = prev_branch.trim();

    let mut git_co = Command::new("git");
    git_co.arg("checkout").arg(branch);
    capture_in_dir(&mut git_co, dir)
        .with_context(|| format!("failed to check out Git branch {branch:?}"))?;

    if prev_branch != branch {
        eprintln!(
            "{}⚠ Warning: switched from branch '{}' to '{}'{}",
            ORANGE, prev_branch, branch, NORMAL
        );
    }
    if !no_pull {
        let mut git_upstream = Command::new("git");
        git_upstream
            .args(["for-each-ref", "--format=%(upstream)"])
            .arg(format!("refs/heads/{branch}"));
        let output = capture_in_dir(&mut git_upstream, dir)
            .with_context(|| format!("failed to query upstream for branch {branch:?}"))?;
        if !output.stdout.iter().all(u8::is_ascii_whitespace) {
            let mut git_pull = Command::new("git");
            git_pull.arg("pull");
            run_in_dir(&mut git_pull, dir)
                .with_context(|| format!("failed to pull Git branch {branch:?}"))?;
        } else {
            eprintln!(
                "Warning: branch {branch:?} has no upstream in {dir:?}; skipping git pull. Configure an upstream or use --no-pull."
            );
        }
    }
    Ok(())
}

/// Locate the host flake directory.
fn host_dir(workdir: &Path, name: &str) -> std::path::PathBuf {
    workdir.join("hosts").join(name)
}

fn check_host(workdir: &Path, name: &str) -> anyhow::Result<()> {
    ensure!(
        !name.is_empty() && Path::new(name).file_name() == Some(std::ffi::OsStr::new(name)),
        "invalid host name {name:?}; expected one directory name under hosts/"
    );
    let flake = host_dir(workdir, name).join("flake.nix");
    let metadata = std::fs::metadata(&flake)
        .with_context(|| format!("cannot access host flake {flake:?} for host {name:?}; check the selected host and repository"))?;
    ensure!(
        metadata.is_file(),
        "host flake {flake:?} is not a regular file"
    );
    Ok(())
}

/// Reference a host flake attribute.
fn host_flake_ref(name: &str, attr: &str) -> String {
    format!("./hosts/{name}#{attr}")
}

/// Update the selected host flake lock.
fn do_flake_update(host_dir: &Path) -> anyhow::Result<()> {
    println!("{}Updating flakes...{}", BLUE, NORMAL);
    let mut cmd = Command::new("nix");
    cmd.arg("flake").arg("update").arg("--commit-lock-file");
    capture_in_dir(&mut cmd, host_dir).context("failed to update the host flake lock")?;
    Ok(())
}

fn do_nix_build(dir: &Path, name: &str, config_attr: &str) -> anyhow::Result<()> {
    let target = format!("./hosts/{name}#{config_attr}.{name}.config.system.build.toplevel");
    let nom_available = Command::new("nom")
        .arg("--version")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    let mut cmd = Command::new(if nom_available { "nom" } else { "nix" });
    cmd.arg("build").arg("-L").arg(&target).arg("--show-trace");
    run_in_dir(&mut cmd, dir)
        .with_context(|| format!("failed to build configuration {target:?}"))?;
    println!("{}Done!{}", BLUE, NORMAL);
    Ok(())
}

#[cfg(target_os = "linux")]
fn do_nixos_rebuild_switch(
    dir: &Path,
    remote: Option<&Remote>,
    native_hostname: &str,
) -> anyhow::Result<()> {
    if let Some(r) = remote {
        let mut cmd = Command::new("nixos-rebuild");
        cmd.arg("switch")
            .arg("--flake")
            .arg(host_flake_ref(&r.remote_name, &r.remote_name))
            .arg("--target-host")
            .arg(&r.target_host)
            .arg("--sudo");
        run_in_dir(&mut cmd, dir).with_context(|| {
            format!(
                "failed to switch remote host {:?} to configuration {:?}",
                r.target_host, r.remote_name
            )
        })?;
    } else {
        let mut sudo_cmd = Command::new("sudo");
        sudo_cmd
            .arg("nixos-rebuild")
            .arg("switch")
            .arg("--flake")
            .arg(host_flake_ref(native_hostname, native_hostname));
        run_in_dir(&mut sudo_cmd, dir).with_context(|| {
            format!("failed to switch local NixOS configuration {native_hostname:?}")
        })?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn do_darwin_rebuild_switch(dir: &Path, hostname: &str) -> anyhow::Result<()> {
    let mut cmd = Command::new("sudo");
    cmd.arg("darwin-rebuild")
        .arg("switch")
        .arg("--flake")
        .arg(host_flake_ref(hostname, hostname));
    run_in_dir(&mut cmd, dir)
        .with_context(|| format!("failed to switch Darwin configuration {hostname:?}"))?;
    Ok(())
}

fn check_repository(dir: &Path, require_clean: bool) -> anyhow::Result<()> {
    let output = capture_in_dir(Command::new("git").args(["status", "--porcelain"]), dir)
        .context("failed to inspect the Nix configuration Git working tree; check NIX_DOT_FILES (or the current directory if unset)")?;
    if require_clean && !output.stdout.is_empty() {
        let dirty = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow::anyhow!(
            "Nix configuration repository {dir:?} is not clean:\n{}\nCommit or stash all changes before switching.",
            dirty.trim()
        ));
    }
    Ok(())
}

fn tag_and_push(dir: &Path, config_name: &str) -> anyhow::Result<()> {
    let date_out = capture_in_dir(Command::new("date").arg("+%Y-%m-%d-%H-%M"), dir)
        .context("failed to get the current date/time for the configuration tag")?;
    let timestamp = String::from_utf8(date_out.stdout)
        .context("date returned non-UTF-8 output for the configuration tag")?;
    let timestamp = timestamp.trim();
    ensure!(
        !timestamp.is_empty(),
        "date returned an empty timestamp for the configuration tag"
    );
    let tag = format!("{}-{}", config_name, timestamp);

    run_in_dir(Command::new("git").arg("tag").arg(&tag), dir)
        .with_context(|| format!("failed to create local Git tag {tag:?}"))?;

    std::thread::sleep(std::time::Duration::from_secs(2));

    run_in_dir(Command::new("git").args(["push", "origin", &tag]), dir)
        .with_context(|| format!("local Git tag {tag:?} was created, but pushing to origin failed; from {dir:?}, retry: git push origin {}", shell_quote(&tag)))?;

    println!("{}Tagged and pushed: {}{}", BLUE, tag, NORMAL);
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
