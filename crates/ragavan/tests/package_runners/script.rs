use super::harness::{bun, development_port, replace_package_script, write_package};
use crate::support::{ENABLED, TestRepository, assert_stdout, ragavan, stderr};

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
fn commands_without_an_isolatable_script_fail_closed() {
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

    write_package(&repository, r#"{"scripts":{"dev":"next build"}}"#);
    for script in ["next build", "vite build", "vite optimize"] {
        replace_package_script(repository.path(), script);
        let unsupported = bun(repository.path(), &["dev"]);
        assert_eq!(
            unsupported.status.code(),
            Some(1),
            "{script}: {unsupported:?}"
        );
        assert!(
            stderr(&unsupported).contains("no stack adapter recognizes it"),
            "{script}: {unsupported:?}"
        );
    }
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
        assert!(error.starts_with("error[script."), "{script}: {output:?}");
        assert!(error.contains("\n\n  help:"), "{script}: {output:?}");
        assert!(!error.contains('\u{1b}'), "{script}: {output:?}");
    }
}

#[test]
fn the_development_server_must_be_the_runner_argument_sink() {
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
fn multiple_development_servers_are_rejected() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite && next dev"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let output = bun(repository.path(), &["dev"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        stderr(&output).contains("more than one recognized development server"),
        "{output:?}"
    );
}
