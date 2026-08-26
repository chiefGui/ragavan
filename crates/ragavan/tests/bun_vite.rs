mod support;

use self::support::{
    ENABLED, TempDirectory, TestRepository, assert_stdout, assert_success, git, ragavan,
    ragavan_command, state_home_variable, stderr, stdout, test_state_home,
};
use std::{
    env, fs,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
};

#[test]
fn vite_ports_are_stable_and_distinct_across_worktrees() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(
        git(
            repository.path(),
            &["config", "extensions.worktreeConfig", "true"],
        ),
        "",
    );
    assert_stdout(
        git(
            repository.path(),
            &[
                "config",
                "--local",
                "ragavan.repositoryId",
                "stable-port-test-repository",
            ],
        ),
        "",
    );
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let main_port = vite_port(repository.path());
    assert_eq!(vite_port(repository.path()), main_port);

    let linked_worktree = repository.add_worktree("linked");
    let linked_port = vite_port(&linked_worktree);
    assert_ne!(linked_port, main_port);

    let moved_worktree = move_worktree(&repository, &linked_worktree, "moved");
    assert_eq!(vite_port(&moved_worktree), linked_port);

    for worktree in [repository.path(), &moved_worktree] {
        assert_stdout(git(worktree, &["status", "--porcelain"]), "");
    }
}

#[test]
fn occupied_vite_ports_are_reassigned_and_remain_stable() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let preferred = vite_port(repository.path());
    let occupied = TcpListener::bind(("localhost", preferred))
        .expect("the preferred port should be available after Bun stops");
    let reassigned = vite_port(repository.path());
    assert_ne!(reassigned, preferred);
    drop(occupied);

    assert_eq!(vite_port(repository.path()), reassigned);
    assert_stdout(git(repository.path(), &["status", "--porcelain"]), "");
}

#[test]
fn supervised_worktrees_own_distinct_stable_ports_until_exit() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);
    let linked_worktree = repository.add_worktree("supervised");
    let bun = FakeBun::waiting();

    let (main_process, main_port) = start_bun(repository.path(), bun.path());
    let (linked_process, linked_port) = start_bun(&linked_worktree, bun.path());
    assert_ne!(main_port, linked_port);

    let duplicate = run_bun(repository.path(), bun.path(), &["dev"]);
    assert_eq!(duplicate.status.code(), Some(1), "{duplicate:?}");
    assert!(stderr(&duplicate).contains("already has an active development process"));

    stop_bun(main_process);
    stop_bun(linked_process);
    assert_eq!(vite_port(repository.path()), main_port);
    assert_eq!(vite_port(&linked_worktree), linked_port);
}

#[test]
fn bun_commands_pass_through_until_isolation_applies() {
    let directory = TempDirectory::new();
    assert_bun_arguments(bun(directory.path(), &["dev"]), &["dev"]);
    assert_bun_arguments(
        bun(directory.path(), &["dev", "--json"]),
        &["dev", "--json"],
    );

    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_bun_arguments(bun(repository.path(), &["dev"]), &["dev"]);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);
    assert_bun_arguments(bun(repository.path(), &["test"]), &["test"]);
}

#[test]
fn bun_exit_status_is_preserved() {
    let directory = TempDirectory::new();
    let bun = FakeBun::exiting(37);
    let output = run_bun(directory.path(), bun.path(), &["test"]);

    assert_eq!(output.status.code(), Some(37), "{output:?}");
    assert_eq!(stdout(&output), "", "{output:?}");
    assert_eq!(stderr(&output), "", "{output:?}");
}

#[test]
fn enabled_repositories_reject_dev_commands_that_cannot_be_isolated() {
    let repository = TestRepository::new();
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let missing_package = bun(repository.path(), &["dev"]);
    assert_eq!(
        missing_package.status.code(),
        Some(1),
        "{missing_package:?}"
    );
    assert!(
        stderr(&missing_package).contains("no package.json"),
        "{missing_package:?}"
    );

    write_package(&repository, r#"{"scripts":{"dev":"next dev"}}"#);
    let unsupported = bun(repository.path(), &["dev"]);
    assert_eq!(unsupported.status.code(), Some(1), "{unsupported:?}");
    assert!(
        stderr(&unsupported).contains("no stack adapter recognizes it"),
        "{unsupported:?}"
    );
}

#[test]
fn explicit_vite_ports_are_rejected_instead_of_overridden() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let output = bun(repository.path(), &["run", "dev", "--port", "4567"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(stderr(&output).contains("explicit `--port`"));
}

#[test]
fn powershell_hook_wraps_bun_without_owning_vite_arguments() {
    let output = ragavan(TempDirectory::new().path(), &["hook", "powershell"]);

    assert_success(&output);
    let hook = stdout(&output);
    assert!(hook.contains("function global:bun"));
    assert!(hook.contains("__run 'bun'"));
    assert!(hook.contains("__RagavanOriginalCommands['bun'].Path"));
    assert!(hook.contains("ErrorAction SilentlyContinue"));
    assert!(!hook.contains("__bun-arguments"));
    assert!(!hook.contains("--port"));
    assert!(!hook.contains("--strictPort"));
}

#[test]
fn outdated_powershell_hooks_fail_with_a_reload_instruction() {
    let output = ragavan(TempDirectory::new().path(), &["__bun-arguments", "dev"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(stdout(&output), "", "{output:?}");
    assert!(stderr(&output).contains("loaded PowerShell integration is outdated"));
    assert!(stderr(&output).contains("ragavan hook powershell"));
}

#[cfg(windows)]
#[test]
fn powershell_hook_is_quiet_when_bun_is_unavailable() {
    let directory = TempDirectory::new();
    let ragavan_executable = Path::new(env!("CARGO_BIN_EXE_ragavan"));
    let ragavan_directory = ragavan_executable
        .parent()
        .expect("Ragavan test executable should have a parent directory");

    let output = Command::new("powershell.exe")
        .current_dir(directory.path())
        .args([
            "-NoProfile",
            "-Command",
            "$hook = ragavan hook powershell | Out-String; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; Invoke-Expression $hook; if ($null -ne (Get-Command bun -CommandType Function -ErrorAction SilentlyContinue)) { exit 1 }; exit 0",
        ])
        .env("PATH", ragavan_directory)
        .output()
        .expect("PowerShell should start");

    assert_success(&output);
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
}

#[cfg(windows)]
#[test]
fn powershell_adapts_bun_dev_and_preserves_other_bun_commands() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let fake_commands = TempDirectory::new();
    fs::write(
        fake_commands.path().join("bun.cmd"),
        "@echo off\r\nfor %%A in (%*) do @echo %%~A\r\n",
    )
    .expect("fake Bun command should be written");

    let ragavan_executable = Path::new(env!("CARGO_BIN_EXE_ragavan"));
    let ragavan_directory = ragavan_executable
        .parent()
        .expect("Ragavan test executable should have a parent directory");
    let mut command_paths = vec![
        fake_commands.path().to_owned(),
        ragavan_directory.to_owned(),
    ];
    command_paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let command_path = env::join_paths(command_paths).expect("test PATH should be valid");

    let output = Command::new("powershell.exe")
        .current_dir(repository.path())
        .args([
            "-NoProfile",
            "-Command",
            "$hook = ragavan hook powershell | Out-String; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; Invoke-Expression $hook; bun dev; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; bun test --watch; exit $LASTEXITCODE",
        ])
        .env("PATH", command_path)
        .env(state_home_variable(), test_state_home(repository.path()))
        .output()
        .expect("PowerShell should start");

    assert_success(&output);
    let stdout = stdout(&output);
    let arguments: Vec<_> = stdout.lines().collect();
    assert_eq!(arguments.first(), Some(&"dev"), "{output:?}");
    assert_eq!(arguments.get(1), Some(&"--port"), "{output:?}");
    let port: u16 = arguments
        .get(2)
        .expect("port argument should exist")
        .parse()
        .expect("port argument should be numeric");
    assert_ne!(port, 0, "{output:?}");
    assert_eq!(arguments.get(3), Some(&"--strictPort"), "{output:?}");
    assert_eq!(arguments.get(4), Some(&"test"), "{output:?}");
    assert_eq!(arguments.get(5), Some(&"--watch"), "{output:?}");
    assert_eq!(arguments.len(), 6, "{output:?}");
}

fn write_package(repository: &TestRepository, contents: &str) {
    fs::write(repository.path().join("package.json"), contents)
        .expect("package.json should be written");
    assert_stdout(git(repository.path(), &["add", "package.json"]), "");
    assert_success(&git(repository.path(), &["commit", "-m", "add package"]));
}

fn move_worktree(repository: &TestRepository, path: &Path, name: &str) -> PathBuf {
    let destination = repository
        .path()
        .parent()
        .expect("test repository should have a parent directory")
        .join(name);
    let output = Command::new("git")
        .current_dir(repository.path())
        .args(["worktree", "move"])
        .arg(path)
        .arg(&destination)
        .output()
        .expect("Git should start");
    assert_success(&output);
    destination
}

fn bun(directory: &Path, arguments: &[&str]) -> Output {
    let bun = FakeBun::printing();
    run_bun(directory, bun.path(), arguments)
}

fn run_bun(directory: &Path, bun: &Path, arguments: &[&str]) -> Output {
    ragavan_command(directory)
        .arg("__run")
        .arg("bun")
        .arg(bun)
        .args(arguments)
        .output()
        .expect("Ragavan should run Bun")
}

fn start_bun(directory: &Path, bun: &Path) -> (Child, u16) {
    let mut child = ragavan_command(directory)
        .arg("__run")
        .arg("bun")
        .arg(bun)
        .arg("dev")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Ragavan should start Bun");
    let stdout = child.stdout.take().expect("Bun stdout should be piped");
    let mut lines = BufReader::new(stdout).lines();
    let arguments: Vec<_> = (0..4)
        .map(|_| {
            lines
                .next()
                .expect("Bun should print every argument")
                .expect("Bun arguments should be readable")
        })
        .collect();
    assert_eq!(arguments[0], "dev");
    assert_eq!(arguments[1], "--port");
    assert_eq!(arguments[3], "--strictPort");
    let port = arguments[2].parse().expect("port should be numeric");

    (child, port)
}

fn stop_bun(mut child: Child) {
    writeln!(
        child.stdin.take().expect("Bun stdin should be piped"),
        "stop"
    )
    .expect("Bun should receive its stop signal");
    let output = child
        .wait_with_output()
        .expect("Ragavan should wait for Bun");
    assert_success(&output);
    assert_eq!(stderr(&output), "", "{output:?}");
}

fn assert_bun_arguments(output: Output, expected: &[&str]) {
    assert_success(&output);
    assert_eq!(stderr(&output), "", "{output:?}");
    assert_eq!(stdout(&output).lines().collect::<Vec<_>>(), expected);
}

fn vite_port(directory: &Path) -> u16 {
    let output = bun(directory, &["dev"]);
    assert_success(&output);
    assert_eq!(stderr(&output), "", "{output:?}");

    let arguments: Vec<_> = stdout(&output).lines().map(str::to_owned).collect();
    assert_eq!(arguments.first().map(String::as_str), Some("dev"));
    assert_eq!(arguments.get(1).map(String::as_str), Some("--port"));
    assert_eq!(arguments.get(3).map(String::as_str), Some("--strictPort"));
    assert_eq!(arguments.len(), 4);
    arguments[2].parse().expect("port should be numeric")
}

struct FakeBun {
    _directory: TempDirectory,
    path: PathBuf,
}

impl FakeBun {
    fn printing() -> Self {
        #[cfg(windows)]
        let (name, contents) = (
            "bun.cmd",
            b"@echo off\r\nfor %%A in (%*) do @echo %%~A\r\n".as_slice(),
        );
        #[cfg(not(windows))]
        let (name, contents) = (
            "bun",
            b"#!/bin/sh\nfor argument do printf '%s\\n' \"$argument\"; done\n".as_slice(),
        );

        Self::create(name, contents)
    }

    fn exiting(code: u8) -> Self {
        #[cfg(windows)]
        let contents = format!("@echo off\r\nexit /b {code}\r\n");
        #[cfg(not(windows))]
        let contents = format!("#!/bin/sh\nexit {code}\n");

        Self::create(
            if cfg!(windows) { "bun.cmd" } else { "bun" },
            contents.as_bytes(),
        )
    }

    fn waiting() -> Self {
        #[cfg(windows)]
        let contents = concat!(
            "@echo off\r\n",
            "for %%A in (%*) do @echo %%~A\r\n",
            "set /p _ragavan_release=\r\n",
        );
        #[cfg(not(windows))]
        let contents = concat!(
            "#!/bin/sh\n",
            "for argument do printf '%s\\n' \"$argument\"; done\n",
            "IFS= read -r _ragavan_release\n",
        );

        Self::create(
            if cfg!(windows) { "bun.cmd" } else { "bun" },
            contents.as_bytes(),
        )
    }

    fn create(name: &str, contents: &[u8]) -> Self {
        let directory = TempDirectory::new();
        let path = directory.path().join(name);
        fs::write(&path, contents).expect("fake Bun should be written");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&path)
                .expect("fake Bun should exist")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).expect("fake Bun should be executable");
        }

        Self {
            _directory: directory,
            path,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}
