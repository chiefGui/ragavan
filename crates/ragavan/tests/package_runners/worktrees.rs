use super::harness::{
    FakeCommand, development_port, run_bun, start_package_runner, stop_runner, write_package,
};
use crate::support::{
    ENABLED, TestRepository, assert_stdout, assert_success, git, ragavan, stderr,
};
use std::{net::TcpListener, path::Path, process::Command};

#[test]
fn ports_are_stable_and_distinct_across_worktrees() {
    let repository = TestRepository::new();
    write_root_package(&repository);
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
fn occupied_ports_are_reassigned_and_remain_stable() {
    let repository = TestRepository::new();
    write_root_package(&repository);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let preferred = development_port(repository.path());
    let occupied = TcpListener::bind(("localhost", preferred))
        .expect("the preferred port should be available after the development server stops");
    let reassigned = development_port(repository.path());
    assert_ne!(reassigned, preferred);
    drop(occupied);

    assert_eq!(development_port(repository.path()), reassigned);
    assert_stdout(git(repository.path(), &["status", "--porcelain"]), "");
}

#[test]
fn supervised_worktrees_own_distinct_stable_ports_until_exit() {
    let repository = TestRepository::new();
    write_root_package(&repository);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);
    let linked_worktree = repository.add_worktree("supervised");
    let bun = FakeCommand::waiting("bun");

    let (main_process, main_port) =
        start_package_runner(repository.path(), "bun", bun.path(), &["dev"]);
    let (linked_process, linked_port) =
        start_package_runner(&linked_worktree, "bun", bun.path(), &["dev"]);
    assert_ne!(linked_port, main_port);

    let duplicate = run_bun(repository.path(), bun.path(), &["dev"]);
    assert_eq!(duplicate.status.code(), Some(1), "{duplicate:?}");
    assert!(stderr(&duplicate).contains("already has an active development process"));

    stop_runner(main_process);
    stop_runner(linked_process);
    assert_eq!(development_port(repository.path()), main_port);
    assert_eq!(development_port(&linked_worktree), linked_port);
}

fn write_root_package(repository: &TestRepository) {
    write_package(repository, r#"{"scripts":{"dev":"vite"}}"#);
}

fn move_worktree(repository: &TestRepository, path: &Path, name: &str) -> std::path::PathBuf {
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
