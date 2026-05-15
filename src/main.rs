use std::env;
use std::path::Path;
use std::process::{Command, ExitStatus};

use clap::Parser;

const BLUE: &str = "\x1b[34m";
const NORMAL: &str = "\x1b[0m";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Update flakes before building
    #[arg(short = 'u', long = "update")]
    update: bool,

    /// Build only; do not run rebuild switch
    #[arg(short = 'b', long = "build-only")]
    build_only: bool,

    /// Remote in format `target_host:remote_name`
    #[cfg(target_os = "linux")]
    #[arg(long = "remote")]
    remote: Option<Remote>,

    /// Git branch to checkout (default: master)
    #[arg(long = "branch", default_value = "master")]
    branch: String,

    /// Skip `git pull` after checkout
    #[arg(long = "no-pull")]
    no_pull: bool,

    /// Hostname to use for darwin-rebuild.
    /// If omitted, auto-detected from darwinConfigurations in flake.
    #[cfg(target_os = "macos")]
    #[arg(long = "hostname")]
    hostname: Option<String>,
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
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
                return Err("remote must be in format target_host:remote_name".into());
            }
            Ok(Remote {
                target_host: parts[0].to_string(),
                remote_name: parts[1].to_string(),
            })
        }
    }
}

#[cfg(target_os = "linux")]
use remote::Remote;

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Determine working directory from NIX_DOT_FILES env var, fallback to current dir
    let nix_dot = env::var("NIX_DOT_FILES").unwrap_or_else(|_| String::from("."));
    let workdir = Path::new(&nix_dot);

    // Require a clean working tree before any operations when we will switch.
    if !cli.build_only {
        check_git_clean(workdir)?;
    }

    // Checkout the specified branch and optionally pull latest changes.
    do_git_checkout_pull(workdir, &cli.branch, cli.no_pull)?;

    if cli.update {
        do_flake_update(workdir)?;
    }

    #[cfg(target_os = "linux")]
    run_nixos(workdir, &cli)?;

    #[cfg(target_os = "macos")]
    run_darwin(workdir, &cli)?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn run_nixos(workdir: &Path, cli: &Cli) -> anyhow::Result<()> {
    let native_hostname = hostname::get()?.to_string_lossy().into_owned();
    let effective_name = if let Some(ref r) = cli.remote {
        r.remote_name.clone()
    } else {
        native_hostname.clone()
    };

    println!("{}Building...{}", BLUE, NORMAL);
    do_nix_build(workdir, &effective_name, "nixosConfigurations")?;

    if cli.build_only {
        return Ok(());
    }

    println!("{}Switching...{}", BLUE, NORMAL);
    do_nixos_rebuild_switch(workdir, cli.remote.as_ref(), &native_hostname)?;

    do_git_tag_and_push(workdir, &effective_name)?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn run_darwin(workdir: &Path, cli: &Cli) -> anyhow::Result<()> {
    let hostname = match &cli.hostname {
        Some(h) => h.clone(),
        None => resolve_darwin_hostname(workdir)?,
    };

    println!("{}Building...{}", BLUE, NORMAL);
    do_nix_build(workdir, &hostname, "darwinConfigurations")?;

    if cli.build_only {
        return Ok(());
    }

    println!("{}Switching...{}", BLUE, NORMAL);
    do_darwin_rebuild_switch(workdir, &hostname)?;

    do_git_tag_and_push(workdir, &hostname)?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn resolve_darwin_hostname(dir: &Path) -> anyhow::Result<String> {
    let output = Command::new("nix")
        .arg("eval")
        .arg(".#darwinConfigurations")
        .arg("--apply")
        .arg("x: builtins.attrNames x")
        .arg("--json")
        .current_dir(dir)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "failed to evaluate darwinConfigurations from flake: {}",
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8(output.stdout)?;
    let names: Vec<String> = serde_json::from_str(&stdout)?;

    match names.len() {
        0 => Err(anyhow::anyhow!(
            "no darwinConfigurations found in flake. Please specify --hostname."
        )),
        1 => {
            println!(
                "{}Auto-detected darwinConfiguration: {}{}",
                BLUE, names[0], NORMAL
            );
            Ok(names[0].clone())
        }
        _ => Err(anyhow::anyhow!(
            "multiple darwinConfigurations found: {}. Please specify --hostname.",
            names.join(", ")
        )),
    }
}

/// Runs a command in the given directory and returns its exit status.
fn run_in_dir(cmd: &mut Command, dir: &Path) -> anyhow::Result<ExitStatus> {
    cmd.current_dir(dir);
    let status = cmd.status()?;
    Ok(status)
}

/// Runs a command in the given directory with stdout/stderr redirected to /dev/null.
fn run_in_dir_capture(cmd: &mut Command, dir: &Path) -> anyhow::Result<ExitStatus> {
    use std::process::Stdio;
    let status = cmd
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    Ok(status)
}

fn do_git_checkout_pull(dir: &Path, branch: &str, no_pull: bool) -> anyhow::Result<()> {
    let mut git_co = Command::new("git");
    git_co.arg("checkout").arg(branch);
    let status = run_in_dir(&mut git_co, dir)?;
    if !status.success() {
        return Err(anyhow::anyhow!("git checkout failed: {}", status));
    }
    if !no_pull {
        let mut git_upstream = Command::new("git");
        git_upstream
            .arg("rev-parse")
            .arg("--abbrev-ref")
            .arg("--symbolic-full-name")
            .arg("@{u}");
        let has_upstream = run_in_dir_capture(&mut git_upstream, dir)?.success();
        if has_upstream {
            let mut git_pull = Command::new("git");
            git_pull.arg("pull");
            let status = run_in_dir(&mut git_pull, dir)?;
            if !status.success() {
                return Err(anyhow::anyhow!("git pull failed: {}", status));
            }
        }
    }
    Ok(())
}

fn do_flake_update(dir: &Path) -> anyhow::Result<()> {
    println!("{}Updating flakes...{}", BLUE, NORMAL);
    let mut cmd = Command::new("nix");
    cmd.arg("flake").arg("update").arg("--commit-lock-file");
    let status = run_in_dir_capture(&mut cmd, dir)?;
    if !status.success() {
        return Err(anyhow::anyhow!("nix flake update failed: {}", status));
    }
    Ok(())
}

fn do_nix_build(dir: &Path, hostname: &str, config_attr: &str) -> anyhow::Result<()> {
    let target = format!(
        ".#{}.{}.config.system.build.toplevel",
        config_attr, hostname
    );
    let mut cmd = Command::new("nix");
    cmd.arg("build").arg("-L").arg(&target).arg("--show-trace");
    let status = run_in_dir(&mut cmd, dir)?;
    if !status.success() {
        return Err(anyhow::anyhow!("nix build failed: {}", status));
    }
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
            .arg(format!(".#{}", r.remote_name))
            .arg("--target-host")
            .arg(&r.target_host)
            .arg("--sudo");
        let status = run_in_dir(&mut cmd, dir)?;
        if !status.success() {
            return Err(anyhow::anyhow!("nixos-rebuild (remote) failed: {}", status));
        }
    } else {
        let mut sudo_cmd = Command::new("sudo");
        sudo_cmd
            .arg("nixos-rebuild")
            .arg("switch")
            .arg("--flake")
            .arg(format!(".#{}", native_hostname));
        let status = run_in_dir(&mut sudo_cmd, dir)?;
        if !status.success() {
            return Err(anyhow::anyhow!("nixos-rebuild failed: {}", status));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn do_darwin_rebuild_switch(dir: &Path, hostname: &str) -> anyhow::Result<()> {
    let mut cmd = Command::new("sudo");
    cmd.arg("darwin-rebuild")
        .arg("switch")
        .arg("--flake")
        .arg(format!(".#{}", hostname));
    let status = run_in_dir(&mut cmd, dir)?;
    if !status.success() {
        return Err(anyhow::anyhow!("darwin-rebuild failed: {}", status));
    }
    Ok(())
}

fn check_git_clean(dir: &Path) -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .current_dir(dir)
        .output()?;
    if !output.status.success() {
        return Err(anyhow::anyhow!("git status check failed"));
    }
    if !output.stdout.is_empty() {
        let dirty = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow::anyhow!(
            "Nix config repository is not clean:\n{}\nCommit or stash all changes before switching.",
            dirty.trim()
        ));
    }
    Ok(())
}

fn do_git_tag_and_push(dir: &Path, config_name: &str) -> anyhow::Result<()> {
    let date_out = Command::new("date").arg("+%Y-%m-%d-%H-%M").output()?;
    if !date_out.status.success() {
        return Err(anyhow::anyhow!("failed to get current date/time"));
    }
    let timestamp = String::from_utf8(date_out.stdout)?.trim().to_string();
    let tag = format!("{}-{}", config_name, timestamp);

    let status = Command::new("git")
        .arg("tag")
        .arg(&tag)
        .current_dir(dir)
        .status()?;
    if !status.success() {
        return Err(anyhow::anyhow!("git tag failed: {}", status));
    }

    let status = Command::new("git")
        .arg("push")
        .arg("origin")
        .arg(&tag)
        .current_dir(dir)
        .status()?;
    if !status.success() {
        return Err(anyhow::anyhow!("git push tag failed: {}", status));
    }

    println!("{}Tagged and pushed: {}{}",  BLUE, tag, NORMAL);
    Ok(())
}
