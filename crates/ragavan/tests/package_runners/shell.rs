#[cfg(windows)]
use super::harness::{normalized_arguments, write_package};
#[cfg(windows)]
use crate::support::{
    ENABLED, TestRepository, assert_stdout, state_home_variable, test_state_home,
};
use crate::support::{TempDirectory, assert_success, ragavan, stderr, stdout};
#[cfg(windows)]
use std::{env, fs, path::Path, process::Command};

#[test]
fn powershell_wraps_package_runners_without_owning_stack_arguments() {
    let output = ragavan(TempDirectory::new().path(), &["hook", "powershell"]);

    assert_success(&output);
    let hook = stdout(&output);
    for command in ["bun", "npm", "pnpm"] {
        assert!(hook.contains(&format!("function global:{command}")));
        assert!(hook.contains(&format!("__run '{command}'")));
        assert!(hook.contains(&format!("__RagavanOriginalCommands['{command}'].Path")));
    }
    assert!(!hook.contains("function global:yarn"));
    assert!(!hook.contains("ExternalScript"));
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
