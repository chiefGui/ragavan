use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

const ENABLED: &str = "Ragavan is enabled for this repository.\n";
const DISABLED: &str = "Ragavan is disabled for this repository.\n";
const PASSTHROUGH_EXIT_CODE: i32 = 10;

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn enrollment_applies_to_existing_and_future_worktrees() {
    let repository = TestRepository::new();
    assert_stdout(
        git(
            repository.path(),
            &["config", "extensions.worktreeConfig", "true"],
        ),
        "",
    );
    let existing_worktree = repository.add_worktree("existing");

    assert_stdout(ragavan(&existing_worktree, &["status"]), DISABLED);
    assert_stdout(ragavan(&existing_worktree, &["enable"]), ENABLED);
    assert_stdout(ragavan(repository.path(), &["status"]), ENABLED);

    let future_worktree = repository.add_worktree("future");
    assert_stdout(ragavan(&future_worktree, &["status"]), ENABLED);
    assert_stdout(ragavan(&future_worktree, &["disable"]), DISABLED);
    assert_stdout(ragavan(&existing_worktree, &["status"]), DISABLED);

    for worktree in [repository.path(), &existing_worktree, &future_worktree] {
        assert_stdout(git(worktree, &["status", "--porcelain"]), "");
    }
}

#[test]
fn enrollment_changes_are_idempotent_and_reversible() {
    let repository = TestRepository::new();

    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);
    let repository_id = stdout(&git(
        repository.path(),
        &["config", "--local", "--get", "ragavan.repositoryId"],
    ));
    assert!(!repository_id.trim().is_empty());

    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);
    assert_stdout(
        git(
            repository.path(),
            &["config", "--local", "--get", "ragavan.repositoryId"],
        ),
        &repository_id,
    );

    assert_stdout(ragavan(repository.path(), &["disable"]), DISABLED);
    assert_stdout(ragavan(repository.path(), &["disable"]), DISABLED);
    assert_stdout(ragavan(repository.path(), &["status"]), DISABLED);

    let repository_id = git(
        repository.path(),
        &["config", "--local", "--get", "ragavan.repositoryId"],
    );
    assert_eq!(repository_id.status.code(), Some(1), "{repository_id:?}");
}

#[test]
fn enrollment_does_not_apply_to_other_repositories() {
    let enrolled_repository = TestRepository::new();
    let other_repository = TestRepository::new();

    assert_stdout(ragavan(enrolled_repository.path(), &["enable"]), ENABLED);
    assert_stdout(ragavan(other_repository.path(), &["status"]), DISABLED);
}

#[test]
fn enrollment_requires_a_git_repository() {
    let directory = TempDirectory::new();
    let output = ragavan(directory.path(), &["enable"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        stderr(&output).contains("could not enable Ragavan for the repository"),
        "{output:?}"
    );
}

#[test]
fn missing_git_is_reported() {
    let directory = TempDirectory::new();
    let output = Command::new(env!("CARGO_BIN_EXE_ragavan"))
        .current_dir(directory.path())
        .arg("status")
        .env("PATH", "")
        .output()
        .expect("Ragavan should start");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        stderr(&output).contains("could not start Git to read the repository enrollment"),
        "{output:?}"
    );
}

#[test]
fn unknown_commands_are_rejected_before_repository_access() {
    let directory = TempDirectory::new();
    let output = ragavan(directory.path(), &["launch"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(stderr(&output).contains("unrecognized subcommand 'launch'"));
    assert!(stderr(&output).contains("Usage: ragavan"));
}

#[test]
fn unsupported_shells_are_rejected_before_installation() {
    let directory = TempDirectory::new();
    let output = ragavan(directory.path(), &["install", "bash"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(stderr(&output).contains("invalid value 'bash'"));
    assert!(stderr(&output).contains("possible values: powershell"));
}

#[test]
fn automatic_shell_selection_refuses_to_guess() {
    let directory = TempDirectory::new();
    let output = ragavan(directory.path(), &["install"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(stderr(&output).contains("could not detect a supported current shell"));
    assert!(stderr(&output).contains("with `powershell`"));
}

#[test]
fn powershell_can_be_selected_explicitly() {
    let directory = TempDirectory::new();
    let output = Command::new(env!("CARGO_BIN_EXE_ragavan"))
        .current_dir(directory.path())
        .args(["install", "powershell"])
        .env("PATH", "")
        .output()
        .expect("Ragavan should start");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(stderr(&output).contains("could not find PowerShell"));
    assert!(!stderr(&output).contains("Usage: ragavan <COMMAND>"));
}

#[test]
fn no_command_prints_help_before_repository_access() {
    let directory = TempDirectory::new();
    let output = ragavan(directory.path(), &[]);

    assert_success(&output);
    assert!(stdout(&output).contains("Usage: ragavan"));
    assert!(stdout(&output).contains("install"));
    assert!(stdout(&output).contains("uninstall"));
    assert!(stdout(&output).contains("enable"));
    assert!(stdout(&output).contains("--json"));
    assert_eq!(stderr(&output), "");
}

#[test]
fn vite_ports_are_stable_and_distinct_across_worktrees() {
    let repository = TestRepository::new();
    repository.write_package(r#"{"scripts":{"dev":"vite"}}"#);
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

    let moved_worktree = repository.move_worktree(&linked_worktree, "moved");
    assert_eq!(vite_port(&moved_worktree), linked_port);

    for worktree in [repository.path(), &moved_worktree] {
        assert_stdout(git(worktree, &["status", "--porcelain"]), "");
    }
}

#[test]
fn bun_commands_pass_through_until_isolation_applies() {
    let directory = TempDirectory::new();
    assert_passthrough(ragavan(directory.path(), &["__bun-arguments", "dev"]));
    assert_passthrough(ragavan(
        directory.path(),
        &["__bun-arguments", "dev", "--json"],
    ));

    let repository = TestRepository::new();
    repository.write_package(r#"{"scripts":{"dev":"vite"}}"#);
    assert_passthrough(ragavan(repository.path(), &["__bun-arguments", "dev"]));
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);
    assert_passthrough(ragavan(repository.path(), &["__bun-arguments", "test"]));
}

#[test]
fn json_reports_enrollment_with_one_versioned_contract() {
    let repository = TestRepository::new();

    let enabled = ragavan(repository.path(), &["--json", "enable"]);
    assert_success(&enabled);
    assert_eq!(stderr(&enabled), "", "{enabled:?}");
    assert_eq!(
        json_stdout(&enabled),
        serde_json::json!({"schema_version": 1, "enrollment": "enabled"})
    );

    let status = ragavan(repository.path(), &["status", "--json"]);
    assert_success(&status);
    assert_eq!(stderr(&status), "", "{status:?}");
    assert_eq!(
        json_stdout(&status),
        serde_json::json!({"schema_version": 1, "enrollment": "enabled"})
    );

    let disabled = ragavan(repository.path(), &["disable", "--json"]);
    assert_success(&disabled);
    assert_eq!(stderr(&disabled), "", "{disabled:?}");
    assert_eq!(
        json_stdout(&disabled),
        serde_json::json!({"schema_version": 1, "enrollment": "disabled"})
    );
}

#[test]
fn json_errors_preserve_usage_and_operation_failures() {
    let directory = TempDirectory::new();

    let usage = ragavan(directory.path(), &["--json", "launch"]);
    assert_eq!(usage.status.code(), Some(2), "{usage:?}");
    assert_eq!(stdout(&usage), "", "{usage:?}");
    let usage_error = json_stderr(&usage);
    assert_eq!(usage_error["schema_version"], 1);
    assert_eq!(usage_error["error"]["kind"], "usage");
    assert!(
        usage_error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("unrecognized subcommand 'launch'")),
        "{usage:?}"
    );

    let operation = ragavan(directory.path(), &["enable", "--json"]);
    assert_eq!(operation.status.code(), Some(1), "{operation:?}");
    assert_eq!(stdout(&operation), "", "{operation:?}");
    let operation_error = json_stderr(&operation);
    assert_eq!(operation_error["schema_version"], 1);
    assert_eq!(operation_error["error"]["kind"], "operation");
    assert!(
        operation_error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("could not enable Ragavan")),
        "{operation:?}"
    );

    let integration = Command::new(env!("CARGO_BIN_EXE_ragavan"))
        .current_dir(directory.path())
        .args(["install", "powershell", "--json"])
        .env("PATH", "")
        .output()
        .expect("Ragavan should start");
    assert_eq!(integration.status.code(), Some(1), "{integration:?}");
    assert_eq!(stdout(&integration), "", "{integration:?}");
    let integration_error = json_stderr(&integration);
    assert_eq!(integration_error["error"]["kind"], "operation");
    assert!(
        integration_error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("could not find PowerShell")),
        "{integration:?}"
    );
}

#[test]
fn json_does_not_replace_shell_protocol_output() {
    let output = ragavan(
        TempDirectory::new().path(),
        &["hook", "powershell", "--json"],
    );

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(stdout(&output), "", "{output:?}");
    let error = json_stderr(&output);
    assert_eq!(error["error"]["kind"], "usage");
    assert!(
        error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("unavailable for `hook`")),
        "{output:?}"
    );
}

#[test]
fn enabled_repositories_reject_dev_commands_that_cannot_be_isolated() {
    let repository = TestRepository::new();
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let missing_package = ragavan(repository.path(), &["__bun-arguments", "dev"]);
    assert_eq!(
        missing_package.status.code(),
        Some(1),
        "{missing_package:?}"
    );
    assert!(stderr(&missing_package).contains("no package.json"));

    repository.write_package(r#"{"scripts":{"dev":"next dev"}}"#);
    let unsupported = ragavan(repository.path(), &["__bun-arguments", "dev"]);
    assert_eq!(unsupported.status.code(), Some(1), "{unsupported:?}");
    assert!(stderr(&unsupported).contains("this slice recognizes Vite"));
}

#[test]
fn explicit_vite_ports_are_rejected_instead_of_overridden() {
    let repository = TestRepository::new();
    repository.write_package(r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let output = ragavan(
        repository.path(),
        &["__bun-arguments", "run", "dev", "--port", "4567"],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(stderr(&output).contains("explicit `--port`"));
}

#[test]
fn powershell_hook_wraps_bun_without_owning_vite_arguments() {
    let output = ragavan(TempDirectory::new().path(), &["hook", "powershell"]);

    assert_success(&output);
    let hook = stdout(&output);
    assert!(hook.contains("function global:bun"));
    assert!(hook.contains("__bun-arguments"));
    assert!(hook.contains("ErrorAction SilentlyContinue"));
    assert!(!hook.contains("__RAGAVAN_PASSTHROUGH_EXIT_CODE__"));
    assert!(!hook.contains("--port"));
    assert!(!hook.contains("--strictPort"));
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
    repository.write_package(r#"{"scripts":{"dev":"vite"}}"#);
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

struct TestRepository {
    directory: TempDirectory,
    path: PathBuf,
}

impl TestRepository {
    fn new() -> Self {
        let directory = TempDirectory::new();
        let path = directory.path().join("repository");

        let output = Command::new("git")
            .args(["init", "--initial-branch=main"])
            .arg(&path)
            .output()
            .expect("Git should start");
        assert_success(&output);

        assert_stdout(git(&path, &["config", "user.name", "Ragavan Tests"]), "");
        assert_stdout(
            git(
                &path,
                &["config", "user.email", "ragavan-tests@example.invalid"],
            ),
            "",
        );
        let output = git(&path, &["commit", "--allow-empty", "-m", "initial"]);
        assert_success(&output);

        Self { directory, path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn add_worktree(&self, name: &str) -> PathBuf {
        let path = self.directory.path().join(name);
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["worktree", "add", "--detach"])
            .arg(&path)
            .output()
            .expect("Git should start");
        assert_success(&output);
        path
    }

    fn move_worktree(&self, path: &Path, name: &str) -> PathBuf {
        let destination = self.directory.path().join(name);
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["worktree", "move"])
            .arg(path)
            .arg(&destination)
            .output()
            .expect("Git should start");
        assert_success(&output);
        destination
    }

    fn write_package(&self, contents: &str) {
        fs::write(self.path.join("package.json"), contents)
            .expect("package.json should be written");
        assert_stdout(git(&self.path, &["add", "package.json"]), "");
        assert_success(&git(&self.path, &["commit", "-m", "add package"]));
    }
}

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        for _ in 0..100 {
            let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path =
                env::temp_dir().join(format!("ragavan-test-{}-{sequence}", std::process::id()));

            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("could not create test directory {path:?}: {error}"),
            }
        }

        panic!("could not allocate a unique test directory");
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("could not remove test directory {:?}: {error}", self.0);
        }
    }
}

fn ragavan(directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ragavan"))
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("Ragavan should start")
}

fn git(directory: &Path, arguments: &[&str]) -> Output {
    Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("Git should start")
}

fn assert_stdout(output: Output, expected: &str) {
    assert_success(&output);
    assert_eq!(stdout(&output), expected, "{output:?}");
    assert_eq!(stderr(&output), "", "{output:?}");
}

fn assert_passthrough(output: Output) {
    assert_eq!(
        output.status.code(),
        Some(PASSTHROUGH_EXIT_CODE),
        "{output:?}"
    );
    assert_eq!(stdout(&output), "", "{output:?}");
    assert_eq!(stderr(&output), "", "{output:?}");
}

fn vite_port(directory: &Path) -> u16 {
    let output = ragavan(directory, &["__bun-arguments", "dev"]);
    assert_success(&output);
    assert_eq!(stderr(&output), "", "{output:?}");

    let arguments: Vec<_> = stdout(&output).lines().map(str::to_owned).collect();
    assert_eq!(arguments.first().map(String::as_str), Some("--port"));
    assert_eq!(arguments.get(2).map(String::as_str), Some("--strictPort"));
    assert_eq!(arguments.len(), 3);
    arguments[1].parse().expect("port should be numeric")
}

fn assert_success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json_stdout(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("stdout should contain one JSON value")
}

fn json_stderr(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stderr).expect("stderr should contain one JSON value")
}
