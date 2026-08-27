use super::super::harness::{FakeCommand, bun, run_bun, write_package};
use crate::support::{
    ENABLED, TempDirectory, TestRepository, assert_stdout, assert_success, ragavan, stderr, stdout,
};
use std::process::Output;

#[test]
fn commands_pass_through_until_isolation_applies() {
    let directory = TempDirectory::new();
    assert_arguments(bun(directory.path(), &["dev"]), &["dev"]);
    assert_arguments(
        bun(directory.path(), &["dev", "--json"]),
        &["dev", "--json"],
    );

    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_arguments(bun(repository.path(), &["dev"]), &["dev"]);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);
    assert_arguments(bun(repository.path(), &["test"]), &["test"]);
}

#[test]
fn child_exit_status_is_preserved() {
    let directory = TempDirectory::new();
    let bun = FakeCommand::exiting("bun", 37);
    let output = run_bun(directory.path(), bun.path(), &["test"]);

    assert_eq!(output.status.code(), Some(37), "{output:?}");
    assert_eq!(stdout(&output), "", "{output:?}");
    assert_eq!(stderr(&output), "", "{output:?}");
}

fn assert_arguments(output: Output, expected: &[&str]) {
    assert_success(&output);
    assert_eq!(stderr(&output), "", "{output:?}");
    assert_eq!(stdout(&output).lines().collect::<Vec<_>>(), expected);
}
