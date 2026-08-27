use super::harness::{FakeCommand, normalized_arguments, write_package};
use crate::support::{
    ENABLED, TestRepository, assert_stdout, ragavan_command, state_home_variable, test_state_home,
};
use crate::support::{TempDirectory, assert_success, ragavan, stdout};
use std::{env, fs, path::Path, process::Command};

#[cfg(windows)]
use crate::support::stderr;
#[test]
fn bash_wraps_package_runners_without_owning_stack_arguments() {
    let output = ragavan(TempDirectory::new().path(), &["hook", "bash"]);

    assert_success(&output);
    let hook = stdout(&output);
    for (index, command) in ["bun", "npm", "pnpm"].iter().enumerate() {
        assert!(hook.contains(&format!("$(builtin type -P {command})")));
        assert!(hook.contains(&format!("function {command}")));
        #[cfg(windows)]
        assert!(hook.contains(&format!(
            "__run '{command}' \"${{BASH}}\" '3' '-c' 'exec \"$0\" \"$@\"' \"${{__RagavanOriginalCommands[{index}]}}\" \"$@\""
        )));
        #[cfg(not(windows))]
        assert!(hook.contains(&format!(
            "__run '{command}' \"${{__RagavanOriginalCommands[{index}]}}\" '0' \"$@\""
        )));
    }
    assert!(!hook.contains("function yarn"));
    assert!(!hook.contains("__bun-arguments"));
    assert!(!hook.contains("--port"));
    assert!(!hook.contains("--strictPort"));
}

#[test]
fn powershell_wraps_package_runners_without_owning_stack_arguments() {
    let output = ragavan(TempDirectory::new().path(), &["hook", "powershell"]);

    assert_success(&output);
    let hook = stdout(&output);
    for command in ["bun", "npm", "pnpm"] {
        assert!(hook.contains(&format!("function global:{command}")));
        assert!(hook.contains(&format!("__run '{command}'")));
        assert!(hook.contains(&format!("__RagavanOriginalCommands['{command}'].Path '0'")));
    }
    assert!(!hook.contains("function global:yarn"));
    assert!(!hook.contains("ExternalScript"));
    assert!(hook.contains("ErrorAction SilentlyContinue"));
    assert!(!hook.contains("__bun-arguments"));
    assert!(!hook.contains("--port"));
    assert!(!hook.contains("--strictPort"));
}

#[cfg(windows)]
#[test]
fn powershell_hook_retains_the_running_native_executable() {
    let directory = TempDirectory::new();
    let native_executable = copied_native_executable(directory.path());
    let output = Command::new("powershell.exe")
        .current_dir(directory.path())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$hook = & $env:RAGAVAN_TEST_EXECUTABLE hook powershell | Out-String; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; Invoke-Expression $hook; [Console]::Out.Write($global:__RagavanExecutable)",
        ])
        .env("RAGAVAN_TEST_EXECUTABLE", &native_executable)
        .output()
        .expect("PowerShell should start");

    assert_success(&output);
    assert_eq!(stdout(&output), native_executable.to_string_lossy());
    assert_eq!(stderr(&output), "");
}

#[cfg(unix)]
#[test]
fn bash_hook_retains_the_running_native_executable() {
    let directory = TempDirectory::new();
    let native_executable = copied_native_executable(directory.path());
    let output = Command::new("bash")
        .current_dir(directory.path())
        .args([
            "--noprofile",
            "--norc",
            "-c",
            "hook=\"$(\"$RAGAVAN_TEST_EXECUTABLE\" hook bash)\" || exit; eval \"$hook\"; printf '%s' \"$__RagavanExecutable\"",
        ])
        .env("RAGAVAN_TEST_EXECUTABLE", &native_executable)
        .output()
        .expect("Bash should start");

    assert_success(&output);
    assert_eq!(stdout(&output), native_executable.to_string_lossy());
    assert_eq!(crate::support::stderr(&output), "");
}

fn copied_native_executable(directory: &Path) -> std::path::PathBuf {
    let native_directory = directory.join("native path").join("Ragavan's");
    fs::create_dir_all(&native_directory).expect("native test directory should be created");
    let native_executable = native_directory.join(if cfg!(windows) {
        "ragavan.exe"
    } else {
        "ragavan"
    });
    fs::copy(env!("CARGO_BIN_EXE_ragavan"), &native_executable)
        .expect("native Ragavan test executable should be copied");
    native_executable
}

#[test]
fn launch_arguments_wrap_without_changing_the_package_command() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);
    let program = FakeCommand::printing("bun");

    let output = ragavan_command(repository.path())
        .arg("__run")
        .arg("bun")
        .arg(program.path())
        .arg("2")
        .args(["launch-one", "launch-two", "dev"])
        .output()
        .expect("Ragavan should run the package manager");

    assert_eq!(
        normalized_arguments(output),
        [
            "launch-one",
            "launch-two",
            "dev",
            "--port",
            "<port>",
            "--strictPort",
        ]
    );
}

#[cfg(windows)]
#[test]
fn powershell_is_quiet_when_package_runners_are_unavailable() {
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
            "$hook = ragavan hook powershell | Out-String; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; Invoke-Expression $hook; foreach ($command in @('bun', 'npm', 'pnpm')) { if ($null -ne (Get-Command $command -CommandType Function -ErrorAction SilentlyContinue)) { exit 1 } }; exit 0",
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
fn powershell_adapts_package_runners_and_preserves_other_commands() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let fake_commands = TempDirectory::new();
    for command in ["bun", "npm", "pnpm"] {
        fs::write(
            fake_commands.path().join(format!("{command}.cmd")),
            "@echo off\r\nfor %%A in (%*) do @echo %%~A\r\n",
        )
        .expect("fake package-manager command should be written");
    }

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
            "$hook = ragavan hook powershell | Out-String; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; Invoke-Expression $hook; bun dev; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; npm run dev; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; pnpm dev; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; npm test --watch; exit $LASTEXITCODE",
        ])
        .env("PATH", command_path)
        .env(state_home_variable(), test_state_home(repository.path()))
        .output()
        .expect("PowerShell should start");

    assert_eq!(
        normalized_arguments(output),
        [
            "dev",
            "--port",
            "<port>",
            "--strictPort",
            "run",
            "dev",
            "--",
            "--port",
            "<port>",
            "--strictPort",
            "dev",
            "--port",
            "<port>",
            "--strictPort",
            "test",
            "--watch",
        ]
    );
}

#[cfg(unix)]
#[test]
fn bash_adapts_package_runners_and_preserves_other_commands() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let fake_commands = [
        FakeCommand::printing("bun"),
        FakeCommand::printing("npm"),
        FakeCommand::printing("pnpm"),
    ];
    let ragavan_executable = Path::new(env!("CARGO_BIN_EXE_ragavan"));
    let ragavan_directory = ragavan_executable
        .parent()
        .expect("Ragavan test executable should have a parent directory");
    let mut command_paths: Vec<_> = fake_commands
        .iter()
        .map(|command| {
            command
                .path()
                .parent()
                .expect("fake command should have a parent")
                .to_owned()
        })
        .collect();
    command_paths.push(ragavan_directory.to_owned());
    command_paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let command_path = env::join_paths(command_paths).expect("test PATH should be valid");

    let output = Command::new("bash")
        .current_dir(repository.path())
        .args([
            "--noprofile",
            "--norc",
            "-c",
            "eval \"$(ragavan hook bash)\" || exit; bun dev || exit; npm run dev || exit; pnpm dev || exit; npm test --watch",
        ])
        .env("PATH", command_path)
        .env(state_home_variable(), test_state_home(repository.path()))
        .output()
        .expect("Bash should start");

    assert_eq!(
        normalized_arguments(output),
        [
            "dev",
            "--port",
            "<port>",
            "--strictPort",
            "run",
            "dev",
            "--",
            "--port",
            "<port>",
            "--strictPort",
            "dev",
            "--port",
            "<port>",
            "--strictPort",
            "test",
            "--watch",
        ]
    );
}
