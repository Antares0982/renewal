use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};

fn assert_output(output: Output, success: bool, expected: &[&str]) {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.success(), success, "{text}");
    for needle in expected {
        assert!(text.contains(needle), "missing {needle:?} in {text}");
    }
}

#[test]
fn cli_regression() {
    let root = std::env::temp_dir().join(format!("renewal-test-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let bin = root.join("bin");
    let repo = root.join("repo");
    let trace = root.join("commands");
    fs::create_dir(&bin).unwrap();
    fs::create_dir_all(repo.join("hosts/example")).unwrap();
    fs::write(repo.join("hosts/example/flake.nix"), "{}\n").unwrap();
    let shell = Command::new("sh")
        .args(["-c", "command -v sh"])
        .output()
        .unwrap();
    assert!(shell.status.success());
    let shell = String::from_utf8(shell.stdout).unwrap();
    let git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(git.status.success());
    let git = String::from_utf8(git.stdout).unwrap();
    let git = git.trim();
    for args in [
        vec!["init", "-b", "master"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "Initial",
        ],
    ] {
        assert_output(
            Command::new(git)
                .args(args)
                .current_dir(&repo)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap(),
            true,
            &[],
        );
    }
    for (name, body) in [
        ("git", format!("printf 'git %s\\n' \"$*\" >> \"$TRACE\"\nif [ \"$1\" = \"${{FAIL_GIT-}}\" ]; then echo git-test-failure >&2; exit 23; fi\nexec '{git}' \"$@\"")),
        ("nix", "printf 'nix %s\\n' \"$*\" >> \"$TRACE\"\nif [ \"$1\" = \"${FAIL_NIX-}\" ]; then echo nix-test-failure >&2; exit 24; fi".into()),
        ("nixos-rebuild", "exit 0".into()),
        ("sudo", "exit 0".into()),
        ("date", "echo 2026-10-05-12-00".into()),
    ] {
        let path = bin.join(name);
        fs::write(&path, format!("#!{}\n{body}\n", shell.trim())).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run = |args: &[&str], vars: &[(&str, &std::ffi::OsStr)]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_renewal"));
        cmd.args(args)
            .current_dir(&repo)
            .env("NIX_DOT_FILES", &repo)
            .env("PATH", &bin)
            .env("TRACE", &trace)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env_remove("FAIL_GIT")
            .env_remove("FAIL_NIX");
        for (key, value) in vars {
            cmd.env(key, value);
        }
        cmd.output().unwrap()
    };
    let missing = root.join("missing");
    assert_output(
        run(
            &["--completions", "zsh"],
            &[("NIX_DOT_FILES", missing.as_os_str())],
        ),
        true,
        &["#compdef renewal", "--branch", "--no-tag", "--completions"],
    );
    assert!(!trace.exists(), "completion must not execute commands");
    for (path, expected) in [
        (missing.as_os_str(), "cannot access repository directory"),
        (std::ffi::OsStr::new(""), "NIX_DOT_FILES is empty"),
    ] {
        assert_output(
            run(&[], &[("NIX_DOT_FILES", path)]),
            false,
            &[expected, "NIX_DOT_FILES"],
        );
    }
    assert_output(
        run(
            &[],
            &[(
                "NIX_DOT_FILES",
                repo.join("hosts/example/flake.nix").as_os_str(),
            )],
        ),
        false,
        &["not a directory", "NIX_DOT_FILES"],
    );
    for args in [&[][..], &["--build-only"][..]] {
        assert_output(
            run(args, &[("NIX_DOT_FILES", root.as_os_str())]),
            false,
            &[
                "Git working tree",
                "not a git repository",
                "status",
                "NIX_DOT_FILES",
            ],
        );
    }
    assert_output(
        run(&[], &[("PATH", missing.as_os_str())]),
        false,
        &["could not run", "git", "PATH", repo.to_str().unwrap()],
    );
    #[cfg(target_os = "linux")]
    let host_args = ["--remote", "example"];
    #[cfg(target_os = "macos")]
    let host_args = ["--hostname", "example"];
    let build = [&host_args[..], &["--build-only"]].concat();
    assert_output(
        run(&build, &[]),
        true,
        &["no upstream", "skipping git pull", "Done!"],
    );
    fs::write(repo.join("dirty"), "uncommitted").unwrap();
    assert_output(
        run(&host_args, &[]),
        false,
        &["not clean", "dirty", "Commit or stash"],
    );
    assert_output(run(&build, &[]), true, &["Done!"]);
    fs::remove_file(repo.join("dirty")).unwrap();
    for (command, context) in [
        ("status", "Git working tree"),
        ("rev-parse", "current Git branch"),
        ("checkout", "check out Git branch"),
        ("for-each-ref", "query upstream"),
    ] {
        assert_output(
            run(&build, &[("FAIL_GIT", command.as_ref())]),
            false,
            &[context, "git-test-failure", "23", repo.to_str().unwrap()],
        );
    }
    let update = [&build[..], &["--update"]].concat();
    assert_output(
        run(&update, &[("FAIL_NIX", "flake".as_ref())]),
        false,
        &[
            "update the host flake lock",
            "nix-test-failure",
            "24",
            "hosts/example",
        ],
    );
    assert_output(
        run(&build, &[("FAIL_NIX", "build".as_ref())]),
        false,
        &["failed to build configuration", "nix-test-failure", "24"],
    );
    let nom = bin.join("nom");
    fs::write(
        &nom,
        format!(
            "#!{}\nif [ \"$1\" = --version ]; then echo nom-probe; exit 0; fi\nprintf 'nom %s\\n' \"$*\" >> \"$TRACE\"\nexec nix \"$@\"\n",
            shell.trim()
        ),
    )
    .unwrap();
    fs::set_permissions(&nom, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(&trace, "").unwrap();
    let output = run(&build, &[]);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("nom-probe"));
    assert_output(output, true, &["Done!"]);
    let commands = fs::read_to_string(&trace).unwrap();
    let nix_build = commands
        .lines()
        .find(|line| line.starts_with("nix build "))
        .unwrap();
    assert!(commands.contains(&nix_build.replacen("nix", "nom", 1)));
    assert_output(
        run(&build, &[("FAIL_NIX", "build".as_ref())]),
        false,
        &["failed to build configuration", "nix-test-failure", "24"],
    );
    for mode in [0o644, 0o755] {
        fs::write(&nom, format!("#!{}\nexit 1\n", shell.trim())).unwrap();
        fs::set_permissions(&nom, fs::Permissions::from_mode(mode)).unwrap();
        fs::write(&trace, "").unwrap();
        assert_output(run(&build, &[]), true, &["Done!"]);
        assert!(fs::read_to_string(&trace).unwrap().contains("nix build "));
    }
    fs::remove_file(nom).unwrap();
    assert_output(
        run(&host_args, &[("FAIL_GIT", "push".as_ref())]),
        false,
        &[
            "system switch succeeded",
            "was created",
            "git-test-failure",
            "retry: git push origin 'example-2026-10-05-12-00'",
        ],
    );
    assert_output(
        Command::new(git)
            .args(["tag", "--list"])
            .current_dir(&repo)
            .output()
            .unwrap(),
        true,
        &["example-2026-10-05-12-00"],
    );
    fs::remove_file(repo.join("hosts/example/flake.nix")).unwrap();
    assert_output(
        run(&build, &[]),
        false,
        &["cannot access host flake", "hosts/example/flake.nix"],
    );
    fs::remove_dir_all(root).unwrap();
}
