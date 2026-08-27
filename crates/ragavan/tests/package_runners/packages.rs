use super::harness::{
    FakeCommand, development_port, development_port_for, package_runner, start_package_runner,
    stop_runner,
};
use crate::support::{
    ENABLED, TestRepository, assert_stdout, assert_success, git, ragavan, stderr, stdout,
};
use std::{fs, path::PathBuf};

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
        r#"{"name":"@workspace/web","scripts":{"dev":"next dev","start":"next start"}}"#,
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
