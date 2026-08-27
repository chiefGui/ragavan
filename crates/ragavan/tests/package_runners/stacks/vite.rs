use super::super::harness::{
    bun, development_port, normalized_arguments, package_runner, replace_package_script,
    write_package,
};
use crate::support::{ENABLED, TestRepository, assert_stdout, git, ragavan, stderr};

#[test]
fn vite_plus_after_setup_commands_receives_the_managed_port() {
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
fn vite_server_commands_receive_the_managed_port() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    for script in [
        "vite",
        "vite dev",
        "vite serve",
        "vite preview",
        "vite ./web",
        "vite build && vite",
        "vite optimize && vite preview",
    ] {
        replace_package_script(repository.path(), script);
        assert_eq!(
            normalized_arguments(bun(repository.path(), &["dev"])),
            ["dev", "--port", "<port>", "--strictPort"],
            "{script}"
        );
    }
}

#[test]
fn explicit_vite_ports_are_rejected_instead_of_overridden() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"vite --port=4567"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let scripted = bun(repository.path(), &["dev"]);
    assert_eq!(scripted.status.code(), Some(1), "{scripted:?}");
    assert!(stderr(&scripted).contains("explicit `--port`"));

    replace_package_script(repository.path(), "vite");
    for (command, arguments) in [
        ("bun", &["dev", "--port", "3456"][..]),
        ("npm", &["run", "dev", "--", "--port", "4567"][..]),
        ("pnpm", &["dev", "--port=5678"][..]),
    ] {
        let output = package_runner(repository.path(), command, arguments);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(stderr(&output).contains("explicit `--port`"), "{output:?}");
    }
}

#[test]
fn explicit_vite_plus_ports_are_rejected_instead_of_overridden() {
    let repository = TestRepository::new();
    write_package(
        &repository,
        r#"{"scripts":{"dev":"bun run prepare && vp dev --port=6789"}}"#,
    );
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    let output = bun(repository.path(), &["dev"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(stderr(&output).contains("explicit `--port`"), "{output:?}");
}
