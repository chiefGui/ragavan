mod support;

use self::support::{
    ENABLED, TempDirectory, TestRepository, assert_stdout, assert_success, git, ragavan, stderr,
    stdout,
};
use std::{env, process::Command};

const DISABLED: &str = "Ragavan is disabled for this repository.\n";

#[test]
fn application_repository_queries_target_an_explicit_directory() {
    let repository = TestRepository::new();
    assert_stdout(
        git(
            repository.path(),
            &["config", "--local", "ragavan.enabled", "true"],
        ),
        "",
    );

    let enrollment = ragavan_application::repository_status(repository.path())
        .expect("the application should inspect the selected repository");

    assert_eq!(enrollment, ragavan_application::Enrollment::Enabled);
}

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
    assert!(stderr(&output).starts_with("error[git.worktree.required]:"));
    assert!(
        stderr(&output).contains("the selected directory is not in a Git worktree"),
        "{output:?}"
    );
}

#[test]
fn enrollment_validation_precedes_repository_identity_creation() {
    let directory = TempDirectory::new();
    let bare_repository = directory.path().join("bare.git");
    let initialized = Command::new("git")
        .args(["init", "--bare"])
        .arg(&bare_repository)
        .output()
        .expect("Git should start");
    assert_success(&initialized);

    let output = ragavan(&bare_repository, &["enable"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(stderr(&output).starts_with("error[git.worktree.required]:"));
    let repository_id = Command::new("git")
        .arg("--git-dir")
        .arg(&bare_repository)
        .args(["config", "--local", "--get", "ragavan.repositoryId"])
        .output()
        .expect("Git should start");
    assert_eq!(repository_id.status.code(), Some(1), "{repository_id:?}");
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
    assert!(stderr(&output).starts_with("error[git.start]:"));
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
    assert!(stderr(&output).starts_with("error[cli.command.invalid]:"));
    assert!(stderr(&output).contains("unrecognized subcommand 'launch'"));
    assert!(stderr(&output).contains("help: run `ragavan --help`"));
    assert!(!stderr(&output).contains('\u{1b}'));
}

#[test]
fn unsupported_shells_are_rejected_before_installation() {
    let directory = TempDirectory::new();
    let output = ragavan(directory.path(), &["install", "zsh"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(stderr(&output).starts_with("error[cli.value.invalid]:"));
    assert!(stderr(&output).contains("invalid value 'zsh'"));
    assert!(stderr(&output).contains("possible values: bash, powershell"));
}

#[test]
fn automatic_shell_selection_refuses_to_guess() {
    let directory = TempDirectory::new();
    let output = ragavan(directory.path(), &["install"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(stderr(&output).starts_with("error[shell.unsupported]:"));
    assert!(stderr(&output).contains("could not detect a supported current shell"));
    assert!(stderr(&output).contains("with `bash` or `powershell`"));
}

#[cfg(unix)]
#[test]
fn automatic_shell_selection_installs_the_parent_bash() {
    let home = TempDirectory::new();
    let output = Command::new("bash")
        .current_dir(home.path())
        .args([
            "--noprofile",
            "--norc",
            "-c",
            "\"$RAGAVAN_TEST_EXECUTABLE\" install; status=$?; :; exit $status",
        ])
        .env("HOME", home.path())
        .env("RAGAVAN_TEST_EXECUTABLE", env!("CARGO_BIN_EXE_ragavan"))
        .output()
        .expect("Bash should start");

    assert_success(&output);
    assert!(stdout(&output).contains("installed for Bash"));
    for profile in [".bashrc", ".profile"] {
        assert!(
            std::fs::read_to_string(home.path().join(profile))
                .expect("Bash profile should be installed")
                .contains("ragavan hook bash")
        );
    }
}

#[cfg(unix)]
#[test]
fn installed_bash_integration_loads_in_login_and_non_login_sessions() {
    let home = TempDirectory::new();
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_ragavan"));
    let executable_directory = executable
        .parent()
        .expect("Ragavan should have an executable directory");
    std::fs::write(
        home.path().join(".profile"),
        "PATH=\"$RAGAVAN_TEST_BIN:$PATH\"\n",
    )
    .expect("test login profile should be written");

    let installed = bash_integration(home.path(), &["install", "bash"]);
    assert_success(&installed);

    let path = env::join_paths(
        std::iter::once(executable_directory.to_owned())
            .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
    )
    .expect("test PATH should be valid");

    for arguments in [
        vec!["--login", "-c", "test -n \"${__RagavanExecutable:-}\""],
        vec![
            "--noprofile",
            "-i",
            "-c",
            "test -n \"${__RagavanExecutable:-}\"",
        ],
    ] {
        let output = Command::new("bash")
            .args(arguments)
            .env("HOME", home.path())
            .env("PATH", &path)
            .env("RAGAVAN_TEST_BIN", executable_directory)
            .output()
            .expect("Bash should start");
        assert_success(&output);
    }
}

#[test]
fn bash_installation_is_explicit_idempotent_and_reversible() {
    let home = TempDirectory::new();
    let profile = home.path().join(".bashrc");
    let login_profile = home.path().join(".profile");
    let original = "alias serve=project\n";
    std::fs::write(&profile, original).expect("Bash profile should be written");

    let installed = bash_integration(home.path(), &["install", "bash", "--json"]);
    assert_success(&installed);
    assert_eq!(stderr(&installed), "", "{installed:?}");
    assert_eq!(
        json_stdout(&installed),
        serde_json::json!({
            "integration": {
                "shell": "bash",
                "state": "installed",
                "profiles": [
                    profile.to_string_lossy(),
                    login_profile.to_string_lossy(),
                ],
            },
        })
    );
    let profile_text =
        std::fs::read_to_string(&profile).expect("Bash profile should remain readable");
    assert!(profile_text.starts_with(original));
    assert!(profile_text.contains("ragavan hook bash"));
    assert!(
        std::fs::read_to_string(&login_profile)
            .expect("Bash login profile should remain readable")
            .contains("ragavan hook bash")
    );

    let repeated = bash_integration(home.path(), &["install", "bash"]);
    assert_success(&repeated);
    assert!(stdout(&repeated).contains("already installed for Bash"));
    assert!(stdout(&repeated).contains("eval \"$(ragavan hook bash)\""));

    let uninstalled = bash_integration(home.path(), &["uninstall", "bash", "--json"]);
    assert_success(&uninstalled);
    assert_eq!(
        json_stdout(&uninstalled),
        serde_json::json!({
            "integration": {
                "shell": "bash",
                "state": "uninstalled",
                "profiles": [
                    profile.to_string_lossy(),
                    login_profile.to_string_lossy(),
                ],
            },
        })
    );
    assert_eq!(
        std::fs::read_to_string(&profile).expect("Bash profile should remain readable"),
        original
    );
    assert_eq!(
        std::fs::read_to_string(login_profile).expect("Bash login profile should remain readable"),
        ""
    );

    let repeated = bash_integration(home.path(), &["uninstall", "bash", "--json"]);
    assert_success(&repeated);
    assert_eq!(
        json_stdout(&repeated),
        serde_json::json!({
            "integration": {
                "shell": "bash",
                "state": "already_uninstalled",
                "profiles": [],
            },
        })
    );
}

#[cfg(windows)]
#[test]
fn explicit_bash_installation_uses_the_windows_user_profile() {
    let home = TempDirectory::new();
    let output = Command::new(env!("CARGO_BIN_EXE_ragavan"))
        .current_dir(home.path())
        .args(["install", "bash"])
        .env_remove("HOME")
        .env("USERPROFILE", home.path())
        .output()
        .expect("Ragavan should start");

    assert_success(&output);
    for profile in [".bashrc", ".profile"] {
        assert!(
            std::fs::read_to_string(home.path().join(profile))
                .expect("Bash profile should be installed")
                .contains("ragavan hook bash")
        );
    }
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
    assert!(
        stderr(&output).starts_with("error[shell.powershell.unavailable]:"),
        "{output:?}"
    );
    assert!(stderr(&output).contains("could not find PowerShell"));
    assert!(stderr(&output).contains("help: install `pwsh`"));
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
    assert!(stdout(&output).contains("dashboard"));
    assert!(stdout(&output).contains("--json"));
    assert_eq!(stderr(&output), "");
}

#[test]
fn json_reports_enrollment_states() {
    let repository = TestRepository::new();

    let enabled = ragavan(repository.path(), &["--json", "enable"]);
    assert_success(&enabled);
    assert_eq!(stderr(&enabled), "", "{enabled:?}");
    assert_eq!(
        json_stdout(&enabled),
        serde_json::json!({"enrollment": "enabled"})
    );

    let status = ragavan(repository.path(), &["status", "--json"]);
    assert_success(&status);
    assert_eq!(stderr(&status), "", "{status:?}");
    assert_eq!(
        json_stdout(&status),
        serde_json::json!({"enrollment": "enabled"})
    );

    let disabled = ragavan(repository.path(), &["disable", "--json"]);
    assert_success(&disabled);
    assert_eq!(stderr(&disabled), "", "{disabled:?}");
    assert_eq!(
        json_stdout(&disabled),
        serde_json::json!({"enrollment": "disabled"})
    );
}

#[test]
fn json_errors_expose_stable_diagnostics() {
    let directory = TempDirectory::new();

    let usage = ragavan(directory.path(), &["--json", "launch"]);
    assert_eq!(usage.status.code(), Some(2), "{usage:?}");
    assert_eq!(stdout(&usage), "", "{usage:?}");
    let usage_error = json_stderr(&usage);
    assert_eq!(usage_error.as_object().map(serde_json::Map::len), Some(1));
    assert_eq!(usage_error["error"]["code"], "cli.command.invalid");
    assert_eq!(
        usage_error["error"]["message"],
        "unrecognized subcommand 'launch'"
    );
    assert_eq!(
        usage_error["error"]["details"]["invalid_subcommand"],
        "launch"
    );
    assert!(usage_error["error"]["help"].is_string());
    assert_eq!(usage_error["error"]["causes"], serde_json::json!([]));

    let operation = ragavan(directory.path(), &["enable", "--json"]);
    assert_eq!(operation.status.code(), Some(1), "{operation:?}");
    assert_eq!(stdout(&operation), "", "{operation:?}");
    let operation_error = json_stderr(&operation);
    assert_eq!(operation_error["error"]["code"], "git.worktree.required");
    assert_eq!(
        operation_error["error"]["details"]["operation"],
        "enable Ragavan for the repository"
    );
    assert!(
        operation_error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("not in a Git worktree")),
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
    assert_eq!(
        integration_error["error"]["code"],
        "shell.powershell.unavailable"
    );
    assert!(integration_error["error"]["help"].is_string());
    let expected_executables = if cfg!(windows) {
        serde_json::json!(["pwsh.exe", "powershell.exe"])
    } else {
        serde_json::json!(["pwsh", "powershell"])
    };
    assert_eq!(
        integration_error["error"]["details"]["executables"],
        expected_executables
    );
    assert!(
        integration_error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("could not find PowerShell")),
        "{integration:?}"
    );

    let missing_git = Command::new(env!("CARGO_BIN_EXE_ragavan"))
        .current_dir(directory.path())
        .args(["status", "--json"])
        .env("PATH", "")
        .output()
        .expect("Ragavan should start");
    assert_eq!(missing_git.status.code(), Some(1), "{missing_git:?}");
    let missing_git_error = json_stderr(&missing_git);
    assert_eq!(missing_git_error["error"]["code"], "git.start");
    assert_eq!(
        missing_git_error["error"]["details"]["operation"],
        "read the repository enrollment"
    );
    assert!(
        missing_git_error["error"]["causes"]
            .as_array()
            .is_some_and(|causes| !causes.is_empty()),
        "{missing_git:?}"
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
    assert_eq!(error["error"]["code"], "cli.output.unsupported");
    assert!(error["error"]["help"].is_string());
    assert!(
        error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("unavailable for `hook`")),
        "{output:?}"
    );
}

#[test]
fn malformed_shell_protocol_is_actionable() {
    let output = ragavan(TempDirectory::new().path(), &["__run"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(stdout(&output), "", "{output:?}");
    assert!(
        stderr(&output).starts_with("error[shell.protocol.malformed]:"),
        "{output:?}"
    );
    assert!(
        stderr(&output).contains("help: run `ragavan install` again"),
        "{output:?}"
    );
}

fn json_stdout(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("stdout should contain one JSON value")
}

fn json_stderr(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stderr).expect("stderr should contain one JSON value")
}

fn bash_integration(home: &std::path::Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ragavan"))
        .current_dir(home)
        .args(arguments)
        .env("HOME", home)
        .output()
        .expect("Ragavan should start")
}
