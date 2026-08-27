use super::super::harness::{
    bun, normalized_arguments, package_runner, replace_package_script, write_package,
};
use crate::support::{ENABLED, TestRepository, assert_stdout, ragavan, stderr};

#[test]
fn package_runners_deliver_the_managed_port() {
    let repository = TestRepository::new();
    write_package(
        &repository,
        r#"{"scripts":{"dev":"bun run prepare && NODE_ENV=development next dev --turbopack","start":"next build && next start"}}"#,
    );
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    for (command, arguments, expected) in [
        ("bun", &["dev"][..], &["dev", "--port", "<port>"][..]),
        (
            "bun",
            &["run", "dev"][..],
            &["run", "dev", "--port", "<port>"][..],
        ),
        (
            "npm",
            &["run", "dev"][..],
            &["run", "dev", "--", "--port", "<port>"][..],
        ),
        (
            "npm",
            &["start"][..],
            &["start", "--", "--port", "<port>"][..],
        ),
        ("pnpm", &["dev"][..], &["dev", "--port", "<port>"][..]),
        (
            "pnpm",
            &["run", "start"][..],
            &["run", "start", "--port", "<port>"][..],
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
fn default_development_forms_receive_the_managed_port() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"next"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    for script in ["next", "next --turbopack", "next ./web --webpack"] {
        replace_package_script(repository.path(), script);
        assert_eq!(
            normalized_arguments(bun(repository.path(), &["dev"])),
            ["dev", "--port", "<port>"],
            "{script}"
        );
    }
}

#[test]
fn explicit_ports_are_rejected_instead_of_overridden() {
    let repository = TestRepository::new();
    write_package(&repository, r#"{"scripts":{"dev":"next dev"}}"#);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);

    for script in [
        "next --port 4567",
        "next --port=4567",
        "next -p 4567",
        "next -p4567",
        "next dev --port 4567",
        "next dev --port=4567",
        "next dev -p 4567",
        "next dev -p=4567",
        "next dev -p4567",
        "NODE_ENV=development PORT=4567 next dev",
        "PORT='4567' next dev",
    ] {
        replace_package_script(repository.path(), script);
        let output = bun(repository.path(), &["dev"]);
        assert_eq!(output.status.code(), Some(1), "{script}: {output:?}");
        assert!(
            stderr(&output).contains("explicit Next.js port"),
            "{script}: {output:?}"
        );
    }

    replace_package_script(repository.path(), "next dev");
    for (command, arguments) in [
        ("bun", &["dev", "-p", "4567"][..]),
        ("npm", &["run", "dev", "--", "--port=4567"][..]),
        ("pnpm", &["dev", "-p4567"][..]),
    ] {
        let output = package_runner(repository.path(), command, arguments);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(
            stderr(&output).contains("explicit Next.js port"),
            "{output:?}"
        );
    }
}
