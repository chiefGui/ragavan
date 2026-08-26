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

    let main_port = development_port(repository.path());
    assert_eq!(development_port(repository.path()), main_port);

    let linked_worktree = repository.add_worktree("linked");
    let linked_port = development_port(&linked_worktree);
    assert_ne!(linked_port, main_port);

    let moved_worktree = move_worktree(&repository, &linked_worktree, "moved");
    assert_eq!(development_port(&moved_worktree), linked_port);

    for worktree in [repository.path(), &moved_worktree] {
        assert_stdout(git(worktree, &["status", "--porcelain"]), "");
    }
}

#[test]
fn occupied_vite_ports_are_reassigned_and_remain_stable() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let preferred = development_port(repository.path());
    let occupied = TcpListener::bind(("localhost", preferred))
        .expect("the preferred port should be available after Bun stops");
    let reassigned = development_port(repository.path());
    assert_ne!(reassigned, preferred);
    drop(occupied);

    assert_eq!(development_port(repository.path()), reassigned);
    assert_stdout(git(repository.path(), &["status", "--porcelain"]), "");
}

#[test]
fn supervised_worktrees_own_distinct_stable_ports_until_exit() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);
    let linked_worktree = repository.add_worktree("supervised");
    let bun = FakeCommand::waiting("bun");

    let (main_process, main_port) = start_bun(repository.path(), bun.path());
    let (linked_process, linked_port) = start_bun(&linked_worktree, bun.path());
    assert_ne!(main_port, linked_port);

    let duplicate = run_bun(repository.path(), bun.path(), &["dev"]);
    assert_eq!(duplicate.status.code(), Some(1), "{duplicate:?}");
    assert!(stderr(&duplicate).contains("already has an active development process"));

    stop_runner(main_process);
    stop_runner(linked_process);
    assert_eq!(development_port(repository.path()), main_port);
    assert_eq!(development_port(&linked_worktree), linked_port);
}

#[test]
fn packages_own_distinct_stable_ports_across_runners_and_scripts() {
    let repository = TestRepository::new();
    let web = repository.path().join("apps/web");
    let web_source = web.join("src");
    let api = repository.path().join("apps/api");
    fs::create_dir_all(&web_source).expect("the web package should be created");
    fs::create_dir_all(&api).expect("the API package should be created");
    let package = r#"{"scripts":{"dev":"vite","start":"vite"}}"#;
    fs::write(repository.path().join("package.json"), package)
        .expect("the root package should be written");
    fs::write(web.join("package.json"), package).expect("the web package should be written");
    fs::write(api.join("package.json"), package).expect("the API package should be written");
    assert_stdout(git(repository.path(), &["add", "package.json", "apps"]), "");
    assert_success(&git(
        repository.path(),
        &["commit", "-m", "add workspace packages"],
    ));
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let bun = FakeCommand::waiting("bun");
    let pnpm = FakeCommand::waiting("pnpm");
    let npm = FakeCommand::waiting("npm");
    let (root_process, root_port) =
        start_package_runner(repository.path(), "bun", bun.path(), &["dev"]);
    let (web_process, web_port) = start_package_runner(&web_source, "pnpm", pnpm.path(), &["dev"]);
    let duplicate = package_runner(&web, "npm", &["start"]);
    assert_eq!(duplicate.status.code(), Some(1), "{duplicate:?}");
    assert!(stderr(&duplicate).contains("already has an active development process"));
    let (api_process, api_port) = start_package_runner(&api, "npm", npm.path(), &["start"]);
    assert_ne!(root_port, web_port);
    assert_ne!(root_port, api_port);
    assert_ne!(web_port, api_port);

    stop_runner(root_process);
    stop_runner(web_process);
    stop_runner(api_process);
    assert_eq!(development_port(repository.path()), root_port);
    assert_eq!(development_port_for(&web, "npm", &["start"]), web_port);
    assert_eq!(development_port_for(&api, "bun", &["dev"]), api_port);
    assert_stdout(git(repository.path(), &["status", "--porcelain"]), "");
}

#[test]
fn workspace_targets_share_the_selected_packages_stable_identity() {
    let repository = TestRepository::new();
    let (web, api, named_worker) = write_workspace(&repository);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let npm = FakeCommand::waiting("npm");
    let pnpm = FakeCommand::waiting("pnpm");
    let (web_process, web_port) = start_package_runner(
        repository.path(),
        "npm",
        npm.path(),
        &["run", "dev", "-w", "apps/web"],
    );
    let duplicate = package_runner(
        repository.path(),
        "pnpm",
        &["--filter=@workspace/web", "start"],
    );
    assert_eq!(duplicate.status.code(), Some(1), "{duplicate:?}");
    assert!(stderr(&duplicate).contains("already has an active development process"));

    let (api_process, api_port) = start_package_runner(
        repository.path(),
        "pnpm",
        pnpm.path(),
        &["run", "--filter", "@workspace/api", "start"],
    );
    assert_ne!(web_port, api_port);

    stop_runner(web_process);
    stop_runner(api_process);
    assert_eq!(
        development_port_for(
            repository.path(),
            "pnpm",
            &["--filter", "@workspace/web", "dev"],
        ),
        web_port
    );
    assert_eq!(
        development_port_for(
            &repository.path().join("apps"),
            "pnpm",
            &["--filter", "./web", "dev"],
        ),
        web_port
    );
    assert_eq!(
        development_port_for(
            repository.path(),
            "pnpm",
            &["-r", "--filter", "@workspace/web", "dev"],
        ),
        web_port
    );
    assert_eq!(development_port_for(&web, "bun", &["dev"]), web_port);
    assert_eq!(development_port_for(&api, "npm", &["start"]), api_port);
    let worker_port = development_port_for(repository.path(), "pnpm", &["-F", "worker", "dev"]);
    assert_eq!(
        development_port_for(&named_worker, "bun", &["dev"]),
        worker_port
    );
    let group_port =
        development_port_for(repository.path(), "pnpm", &["--filter", "./group", "dev"]);
    assert_eq!(
        development_port_for(&repository.path().join("group"), "bun", &["dev"]),
        group_port
    );
    assert_stdout(git(repository.path(), &["status", "--porcelain"]), "");
}

#[test]
fn workspace_targets_that_are_not_exactly_one_package_fail_closed() {
    let repository = TestRepository::new();
    write_workspace(&repository);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    for (command, arguments, expected) in [
        (
            "npm",
            &["--workspaces", "run", "dev"][..],
            "may select multiple packages",
        ),
        (
            "npm",
            &["--workspace", "apps", "run", "dev"][..],
            "identifies multiple packages",
        ),
        (
            "npm",
            &["--workspace", "group", "run", "dev"][..],
            "identifies multiple packages",
        ),
        (
            "npm",
            &["--workspace", "worker", "run", "dev"][..],
            "identifies multiple packages",
        ),
        (
            "npm",
            &["--workspace", "apps/web", "--iwr", "run", "dev"][..],
            "may select multiple packages",
        ),
        (
            "pnpm",
            &["--filter", "./apps/*", "dev"][..],
            "is not an exact package name or directory",
        ),
        (
            "pnpm",
            &["-r", "run", "dev"][..],
            "may select multiple packages",
        ),
        (
            "pnpm",
            &[
                "--filter",
                "@workspace/web",
                "--filter",
                "@workspace/api",
                "dev",
            ][..],
            "may select multiple packages",
        ),
        (
            "npm",
            &["run", "dev", "--workspace="][..],
            "requires a package selector",
        ),
        (
            "pnpm",
            &["--filter=", "dev"][..],
            "requires a package selector",
        ),
        (
            "pnpm",
            &["--filter", "missing", "dev"][..],
            "does not identify a package",
        ),
        (
            "npm",
            &["--workspace", "..", "run", "dev"][..],
            "points outside Git worktree",
        ),
    ] {
        let output = package_runner(repository.path(), command, arguments);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(stderr(&output).contains(expected), "{output:?}");
        assert_eq!(stdout(&output), "", "{output:?}");
    }

    assert_stdout(git(repository.path(), &["status", "--porcelain"]), "");
}

#[test]
fn vite_plus_after_setup_commands_receives_the_service_port() {
    let repository = TestRepository::new();
    write_package(
        &repository,
        r#"{"scripts":{"dev":"bun run build:ipc:development && vp dev"}}"#,
    );
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let port = development_port(repository.path());
    assert_eq!(development_port(repository.path()), port);
    assert_stdout(git(repository.path(), &["status", "--porcelain"]), "");
}

#[test]
fn static_environment_assignments_and_quoted_paths_are_supported() {
    let repository = TestRepository::new();
    write_package(
        &repository,
        r#"{"scripts":{"dev":"NODE_ENV='development mode' './node_modules/.bin/vite'"}}"#,
    );
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let port = development_port(repository.path());
    assert_eq!(development_port(repository.path()), port);
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
fn npm_and_pnpm_run_vite_with_the_service_port() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite","start":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    for (command, arguments, expected) in [
        (
            "npm",
            &["run", "dev"][..],
            &["run", "dev", "--", "--port", "<port>", "--strictPort"][..],
        ),
        (
            "npm",
            &["run-script", "dev"][..],
            &[
                "run-script",
                "dev",
                "--",
                "--port",
                "<port>",
                "--strictPort",
            ][..],
        ),
        (
            "npm",
            &["start"][..],
            &["start", "--", "--port", "<port>", "--strictPort"][..],
        ),
        (
            "npm",
            &["run", "start"][..],
            &["run", "start", "--", "--port", "<port>", "--strictPort"][..],
        ),
        (
            "pnpm",
            &["dev"][..],
            &["dev", "--port", "<port>", "--strictPort"][..],
        ),
        (
            "pnpm",
            &["run", "dev"][..],
            &["run", "dev", "--port", "<port>", "--strictPort"][..],
        ),
        (
            "pnpm",
            &["run-script", "dev"][..],
            &["run-script", "dev", "--port", "<port>", "--strictPort"][..],
        ),
        (
            "pnpm",
            &["start"][..],
            &["start", "--port", "<port>", "--strictPort"][..],
        ),
        (
            "pnpm",
            &["run", "start"][..],
            &["run", "start", "--port", "<port>", "--strictPort"][..],
        ),
    ] {
        let output = package_runner(repository.path(), command, arguments);
        assert_eq!(
            normalized_arguments(output),
            expected,
            "{command} {arguments:?}"
        );
    }
}

#[test]
fn npm_reuses_an_existing_script_argument_separator() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let output = package_runner(
        repository.path(),
        "npm",
        &["run", "dev", "--", "--host", "127.0.0.1"],
    );

    assert_eq!(
        normalized_arguments(output),
        [
            "run",
            "dev",
            "--",
            "--host",
            "127.0.0.1",
            "--port",
            "<port>",
            "--strictPort",
        ]
    );
}

#[test]
fn npm_and_pnpm_reject_explicit_vite_ports() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    for (command, arguments) in [
        ("npm", &["run", "dev", "--", "--port", "4567"][..]),
        ("pnpm", &["dev", "--port=5678"][..]),
    ] {
        let output = package_runner(repository.path(), command, arguments);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(stderr(&output).contains("explicit `--port`"), "{output:?}");
    }
}

#[test]
fn unrelated_npm_and_pnpm_commands_take_the_direct_pass_through_path() {
    let directory = TempDirectory::new();

    for (command, arguments) in [
        ("npm", &["install", "example-package"][..]),
        ("npm", &["--workspaces", "run", "dev"][..]),
        ("pnpm", &["test", "--watch"][..]),
        ("pnpm", &["--filter", "@workspace/web...", "dev"][..]),
        ("pnpm", &["-r", "run", "dev"][..]),
    ] {
        let executable = FakeCommand::printing(command);
        let output = ragavan_command(directory.path())
            .arg("__run")
            .arg(command)
            .arg(executable.path())
            .args(arguments)
            .env("PATH", "")
            .output()
            .expect("Ragavan should pass through the package-manager command");

        assert_success(&output);
        assert_eq!(stderr(&output), "", "{output:?}");
        assert_eq!(stdout(&output).lines().collect::<Vec<_>>(), arguments);
    }
}

#[test]
fn bun_exit_status_is_preserved() {
    let directory = TempDirectory::new();
    let bun = FakeCommand::exiting("bun", 37);
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
fn explicit_vite_plus_ports_in_package_scripts_are_rejected() {
    let repository = TestRepository::new();
    write_package(
        &repository,
        r#"{"scripts":{"dev":"bun run prepare && vp dev --port=4567"}}"#,
    );
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let output = bun(repository.path(), &["dev"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(stderr(&output).contains("explicit `--port`"), "{output:?}");
}

#[test]
fn unsupported_shell_syntax_fails_closed() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vp dev | worker"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    for script in [
        "vp dev | worker",
        "vp dev || worker",
        "vp dev & worker",
        "vp dev; worker",
        "$(vp dev)",
        "vp dev # comment",
        "vp dev\nworker",
    ] {
        replace_package_script(repository.path(), script);
        let output = bun(repository.path(), &["dev"]);
        assert_eq!(output.status.code(), Some(1), "{script}: {output:?}");
        let error = stderr(&output);
        assert!(error.contains("unsupported script"), "{script}: {output:?}");
        assert_eq!(error.lines().count(), 1, "{script}: {output:?}");
    }
}

#[test]
fn development_server_must_be_the_runner_argument_sink() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vp dev && echo done"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let output = bun(repository.path(), &["dev"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        stderr(&output).contains("development server must be the final command"),
        "{output:?}"
    );
}

#[test]
fn multiple_recognized_development_servers_are_rejected() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite && vp dev"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let output = bun(repository.path(), &["dev"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        stderr(&output).contains("more than one recognized development server"),
        "{output:?}"
    );
}

#[test]
fn powershell_hook_wraps_package_runners_without_owning_vite_arguments() {
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
fn powershell_hook_is_quiet_when_package_runners_are_unavailable() {
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

fn write_package(repository: &TestRepository, contents: &str) {
    fs::write(repository.path().join("package.json"), contents)
        .expect("package.json should be written");
    assert_stdout(git(repository.path(), &["add", "package.json"]), "");
    assert_success(&git(repository.path(), &["commit", "-m", "add package"]));
}

fn write_workspace(repository: &TestRepository) -> (PathBuf, PathBuf, PathBuf) {
    let web = repository.path().join("apps/web");
    let api = repository.path().join("apps/api");
    let named_worker = repository.path().join("apps/worker");
    let directory_worker = repository.path().join("worker");
    let group = repository.path().join("group");
    let group_child = group.join("child");
    let invalid_fixture = repository.path().join("fixtures/invalid");
    let generated = repository.path().join("generated");
    fs::create_dir_all(&web).expect("the web package should be created");
    fs::create_dir_all(&api).expect("the API package should be created");
    fs::create_dir_all(&named_worker).expect("the named worker package should be created");
    fs::create_dir_all(&directory_worker).expect("the directory worker package should be created");
    fs::create_dir_all(&group_child).expect("the nested group package should be created");
    fs::create_dir_all(&invalid_fixture).expect("the invalid fixture should be created");
    fs::create_dir_all(&generated).expect("the ignored package should be created");
    fs::write(
        repository.path().join(".gitignore"),
        if cfg!(windows) {
            "generated/\r\n"
        } else {
            "generated/\n"
        },
    )
    .expect("the repository exclusions should be written");
    fs::write(
        repository.path().join("package.json"),
        r#"{"name":"workspace-root","private":true,"workspaces":["apps/*","worker","group/**"]}"#,
    )
    .expect("the workspace package should be written");
    fs::write(
        repository.path().join("pnpm-workspace.yaml"),
        if cfg!(windows) {
            "packages:\r\n  - apps/*\r\n  - worker\r\n  - group/**\r\n"
        } else {
            "packages:\n  - apps/*\n  - worker\n  - group/**\n"
        },
    )
    .expect("the pnpm workspace should be written");
    fs::write(
        web.join("package.json"),
        r#"{"name":"@workspace/web","scripts":{"dev":"vite","start":"vite"}}"#,
    )
    .expect("the web package should be written");
    fs::write(
        api.join("package.json"),
        r#"{"name":"@workspace/api","scripts":{"dev":"vite","start":"vite"}}"#,
    )
    .expect("the API package should be written");
    fs::write(
        named_worker.join("package.json"),
        r#"{"name":"worker","scripts":{"dev":"vite"}}"#,
    )
    .expect("the named worker package should be written");
    fs::write(
        directory_worker.join("package.json"),
        r#"{"name":"workspace-directory","scripts":{"dev":"vite"}}"#,
    )
    .expect("the directory worker package should be written");
    fs::write(
        group.join("package.json"),
        r#"{"name":"group","scripts":{"dev":"vite"}}"#,
    )
    .expect("the group package should be written");
    fs::write(
        group_child.join("package.json"),
        r#"{"name":"group-child","scripts":{"dev":"vite"}}"#,
    )
    .expect("the nested group package should be written");
    fs::write(invalid_fixture.join("package.json"), "{not valid JSON")
        .expect("the invalid fixture should be written");
    fs::write(
        generated.join("package.json"),
        r#"{"name":"@workspace/web","scripts":{"dev":"vite"}}"#,
    )
    .expect("the ignored package should be written");
    assert_stdout(
        git(
            repository.path(),
            &[
                "add",
                ".gitignore",
                "package.json",
                "pnpm-workspace.yaml",
                "apps",
                "fixtures",
                "group",
                "worker",
            ],
        ),
        "",
    );
    assert_success(&git(
        repository.path(),
        &["commit", "-m", "add package workspace"],
    ));

    (web, api, named_worker)
}

fn replace_package_script(repository: &Path, script: &str) {
    let package = serde_json::json!({ "scripts": { "dev": script } });
    fs::write(repository.join("package.json"), package.to_string())
        .expect("package.json should be replaced");
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
    let bun = FakeCommand::printing("bun");
    run_bun(directory, bun.path(), arguments)
}

fn package_runner(directory: &Path, command: &str, arguments: &[&str]) -> Output {
    let executable = FakeCommand::printing(command);
    run_package_runner(directory, command, executable.path(), arguments)
}

fn run_package_runner(
    directory: &Path,
    command: &str,
    executable: &Path,
    arguments: &[&str],
) -> Output {
    ragavan_command(directory)
        .arg("__run")
        .arg(command)
        .arg(executable)
        .args(arguments)
        .output()
        .expect("Ragavan should run the package manager")
}

fn normalized_arguments(output: Output) -> Vec<String> {
    assert_success(&output);
    assert_eq!(stderr(&output), "", "{output:?}");

    let mut arguments: Vec<_> = stdout(&output).lines().map(str::to_owned).collect();
    let mut ports = 0;
    for index in 1..arguments.len() {
        if arguments[index - 1] != "--port" {
            continue;
        }
        let port: u16 = arguments[index]
            .parse()
            .expect("Vite's port should be numeric");
        assert_ne!(port, 0);
        arguments[index] = "<port>".to_owned();
        ports += 1;
    }
    assert_ne!(ports, 0, "Vite should receive a port");
    arguments
}

struct FakeCommand {
    _directory: TempDirectory,
    path: PathBuf,
}

impl FakeCommand {
    fn printing(name: &str) -> Self {
        #[cfg(windows)]
        let (name, contents) = (
            format!("{name}.cmd"),
            b"@echo off\r\nfor %%A in (%*) do @echo %%~A\r\n".as_slice(),
        );
        #[cfg(not(windows))]
        let (name, contents) = (
            name.to_owned(),
            b"#!/bin/sh\nfor argument do printf '%s\\n' \"$argument\"; done\n".as_slice(),
        );

        Self::create(&name, contents)
    }

    fn exiting(name: &str, code: u8) -> Self {
        #[cfg(windows)]
        let contents = format!("@echo off\r\nexit /b {code}\r\n");
        #[cfg(not(windows))]
        let contents = format!("#!/bin/sh\nexit {code}\n");

        Self::create(&command_file_name(name), contents.as_bytes())
    }

    fn waiting(name: &str) -> Self {
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

        Self::create(&command_file_name(name), contents.as_bytes())
    }

    fn create(name: &str, contents: &[u8]) -> Self {
        let directory = TempDirectory::new();
        let path = directory.path().join(name);
        fs::write(&path, contents).expect("fake command should be written");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&path)
                .expect("fake command should exist")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).expect("fake command should be executable");
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

fn command_file_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.to_owned()
    }
}

fn run_bun(directory: &Path, bun: &Path, arguments: &[&str]) -> Output {
    run_package_runner(directory, "bun", bun, arguments)
}

fn start_package_runner(
    directory: &Path,
    command: &str,
    executable: &Path,
    arguments: &[&str],
) -> (Child, u16) {
    let mut child = ragavan_command(directory)
        .arg("__run")
        .arg(command)
        .arg(executable)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Ragavan should start the package manager");
    let stdout = child
        .stdout
        .take()
        .expect("package-manager stdout should be piped");
    let mut lines = BufReader::new(stdout).lines();
    let mut delivered_arguments = Vec::new();
    loop {
        let argument = lines
            .next()
            .expect("the package manager should print its arguments")
            .expect("package-manager arguments should be readable");
        let complete = argument == "--strictPort";
        delivered_arguments.push(argument);
        if complete {
            break;
        }
    }
    assert_eq!(
        &delivered_arguments[..arguments.len()],
        arguments,
        "the original runner arguments should be preserved"
    );
    let port = port_from_arguments(&delivered_arguments);

    (child, port)
}

fn start_bun(directory: &Path, bun: &Path) -> (Child, u16) {
    start_package_runner(directory, "bun", bun, &["dev"])
}

fn stop_runner(mut child: Child) {
    writeln!(
        child
            .stdin
            .take()
            .expect("package-manager stdin should be piped"),
        "stop"
    )
    .expect("the package manager should receive its stop signal");
    let output = child
        .wait_with_output()
        .expect("Ragavan should wait for the package manager");
    assert_success(&output);
    assert_eq!(stderr(&output), "", "{output:?}");
}

fn assert_bun_arguments(output: Output, expected: &[&str]) {
    assert_success(&output);
    assert_eq!(stderr(&output), "", "{output:?}");
    assert_eq!(stdout(&output).lines().collect::<Vec<_>>(), expected);
}

fn development_port(directory: &Path) -> u16 {
    development_port_for(directory, "bun", &["dev"])
}

fn development_port_for(directory: &Path, command: &str, arguments: &[&str]) -> u16 {
    let output = package_runner(directory, command, arguments);
    assert_success(&output);
    assert_eq!(stderr(&output), "", "{output:?}");

    let arguments: Vec<_> = stdout(&output).lines().map(str::to_owned).collect();
    port_from_arguments(&arguments)
}

fn port_from_arguments(arguments: &[String]) -> u16 {
    let port_index = arguments
        .iter()
        .position(|argument| argument == "--port")
        .expect("Vite should receive a port")
        + 1;
    assert_eq!(arguments.last().map(String::as_str), Some("--strictPort"));
    arguments
        .get(port_index)
        .expect("Vite's port should have a value")
        .parse()
        .expect("Vite's port should be numeric")
}
