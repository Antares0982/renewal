use std::env;
use std::path::Path;
use std::process::{Command, ExitStatus};

use clap::Parser;
use std::str::FromStr;

const BLUE: &str = "\x1b[34m";
const NORMAL: &str = "\x1b[0m";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Update flakes before building
    #[arg(short = 'u', long = "update")]
    update: bool,

    /// Build only; do not run `nixos-rebuild switch`
    #[arg(short = 'b', long = "build-only")]
    build_only: bool,

    /// Remote in format `target_host:remote_name`
    #[arg(long = "remote")]
    remote: Option<Remote>,
}

#[derive(Debug, Clone)]
struct Remote {
    target_host: String,
    remote_name: String,
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

    // Always perform git checkout master && git pull in the NIX_DOT_FILES dir (like the original script).
    do_git_checkout_pull(workdir)?;

    if cli.update {
        do_flake_update(workdir)?;
    }

    // get hostname once; if remote provided, use its remote_name as hostname
    let native_hostname = hostname::get()?.to_string_lossy().into_owned();
    let effective_name = if let Some(ref r) = cli.remote {
        r.remote_name.clone()
    } else {
        native_hostname.clone()
    };

    // perform build
    println!("{}Building...{}", BLUE, NORMAL);
    do_nix_build(workdir, &effective_name)?;

    if cli.build_only {
        // build-only: stop here
        return Ok(());
    }

    // switch
    println!("{}Switching...{}", BLUE, NORMAL);
    do_nixos_rebuild_switch(workdir, cli.remote.as_ref(), &native_hostname)?;

    // If neither flag matched (or after processing), exit normally.
    Ok(())
}

fn run_in_dir(cmd: &mut Command, dir: &Path) -> anyhow::Result<ExitStatus> {
    cmd.current_dir(dir);
    let status = cmd.status()?;
    Ok(status)
}

fn run_in_dir_capture(cmd: &mut Command, dir: &Path) -> anyhow::Result<ExitStatus> {
    use std::fs::File;
    // Redirect stdout/stderr to /dev/null to mimic original script
    let devnull = File::open("/dev/null")?;
    cmd.current_dir(dir)
        .stdout(devnull.try_clone()?)
        .stderr(devnull);
    let status = cmd.status()?;
    Ok(status)
}

fn do_git_checkout_pull(dir: &Path) -> anyhow::Result<()> {
    let mut git_co = Command::new("git");
    git_co.arg("checkout").arg("master");
    let status = run_in_dir(&mut git_co, dir)?;
    if !status.success() {
        return Err(anyhow::anyhow!("git checkout failed: {}", status));
    }
    let mut git_pull = Command::new("git");
    git_pull.arg("pull");
    let status = run_in_dir(&mut git_pull, dir)?;
    if !status.success() {
        return Err(anyhow::anyhow!("git pull failed: {}", status));
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

fn do_nix_build(dir: &Path, hostname: &str) -> anyhow::Result<()> {
    let target = format!(
        ".#nixosConfigurations.{}.config.system.build.toplevel",
        hostname
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

fn do_nixos_rebuild_switch(
    dir: &Path,
    remote: Option<&Remote>,
    native_hostname: &str,
) -> anyhow::Result<()> {
    if let Some(r) = remote {
        // Remote rebuild: do not use sudo, use --target-host and --sudo
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
