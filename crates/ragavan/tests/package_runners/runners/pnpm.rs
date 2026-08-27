use super::super::harness::{FakeCommand, normalized_arguments, package_runner, write_package};
use crate::support::{
    ENABLED, TempDirectory, TestRepository, assert_stdout, assert_success, ragavan,
    ragavan_command, stderr, stdout,
};

#[test]
fn development_scripts_receive_stack_arguments() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite","start":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    for (arguments, expected) in [
        (
            &["dev"][..],
            &["dev", "--port", "<port>", "--strictPort"][..],
        ),
        (
            &["run", "dev"][..],
            &["run", "dev", "--port", "<port>", "--strictPort"][..],
        ),
        (
            &["run-script", "dev"][..],
            &["run-script", "dev", "--port", "<port>", "--strictPort"][..],
        ),
        (
            &["start"][..],
            &["start", "--port", "<port>", "--strictPort"][..],
        ),
        (
            &["run", "start"][..],
            &["run", "start", "--port", "<port>", "--strictPort"][..],
        ),
    ] {
        assert_eq!(
            normalized_arguments(package_runner(repository.path(), "pnpm", arguments)),
            expected,
            "pnpm {arguments:?}"
        );
    }
}

#[test]
fn unrelated_commands_take_the_direct_pass_through_path() {
    let directory = TempDirectory::new();

    for arguments in [
        &["test", "--watch"][..],
        &["--filter", "@workspace/web...", "dev"][..],
        &["-r", "run", "dev"][..],
    ] {
        let executable = FakeCommand::printing("pnpm");
        let output = ragavan_command(directory.path())
            .arg("__run")
            .arg("pnpm")
            .arg(executable.path())
            .args(arguments)
            .env("PATH", "")
            .output()
            .expect("Ragavan should pass through pnpm");

        assert_success(&output);
        assert_eq!(stderr(&output), "", "{output:?}");
        assert_eq!(stdout(&output).lines().collect::<Vec<_>>(), arguments);
    }
}
