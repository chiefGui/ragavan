use crate::support::{
    TempDirectory, TestRepository, assert_stdout, assert_success, git, ragavan_command, stderr,
    stdout,
};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Output, Stdio},
};

const ARGUMENTS_END: &str = "__RAGAVAN_TEST_ARGUMENTS_END__";

pub(super) fn write_package(repository: &TestRepository, contents: &str) {
    fs::write(repository.path().join("package.json"), contents)
        .expect("package.json should be written");
    assert_stdout(git(repository.path(), &["add", "package.json"]), "");
    assert_success(&git(repository.path(), &["commit", "-m", "add package"]));
}

pub(super) fn replace_package_script(repository: &Path, script: &str) {
    let package = serde_json::json!({ "scripts": { "dev": script } });
    fs::write(repository.join("package.json"), package.to_string())
        .expect("package.json should be replaced");
}

pub(super) fn bun(directory: &Path, arguments: &[&str]) -> Output {
    let bun = FakeCommand::printing("bun");
    run_bun(directory, bun.path(), arguments)
}

pub(super) fn package_runner(directory: &Path, command: &str, arguments: &[&str]) -> Output {
    let executable = FakeCommand::printing(command);
    run_package_runner(directory, command, executable.path(), arguments)
}

pub(super) fn run_package_runner(
    directory: &Path,
    command: &str,
    executable: &Path,
    arguments: &[&str],
) -> Output {
    ragavan_command(directory)
        .arg("__run")
        .arg(command)
        .arg(executable)
        .arg("0")
        .args(arguments)
        .output()
        .expect("Ragavan should run the package manager")
}

pub(super) fn normalized_arguments(output: Output) -> Vec<String> {
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
            .expect("the managed port should be numeric");
        assert_ne!(port, 0);
        arguments[index] = "<port>".to_owned();
        ports += 1;
    }
    assert_ne!(ports, 0, "the development server should receive a port");
    arguments
}

pub(super) struct FakeCommand {
    _directory: TempDirectory,
    path: PathBuf,
}

impl FakeCommand {
    pub(super) fn printing(name: &str) -> Self {
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

    pub(super) fn exiting(name: &str, code: u8) -> Self {
        #[cfg(windows)]
        let contents = format!("@echo off\r\nexit /b {code}\r\n");
        #[cfg(not(windows))]
        let contents = format!("#!/bin/sh\nexit {code}\n");

        Self::create(&command_file_name(name), contents.as_bytes())
    }

    pub(super) fn waiting(name: &str) -> Self {
        #[cfg(windows)]
        let contents = concat!(
            "@echo off\r\n",
            "for %%A in (%*) do @echo %%~A\r\n",
            "echo __RAGAVAN_TEST_ARGUMENTS_END__\r\n",
            "set /p _ragavan_release=\r\n",
        );
        #[cfg(not(windows))]
        let contents = concat!(
            "#!/bin/sh\n",
            "for argument do printf '%s\\n' \"$argument\"; done\n",
            "printf '%s\\n' '__RAGAVAN_TEST_ARGUMENTS_END__'\n",
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

    pub(super) fn path(&self) -> &Path {
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

pub(super) fn run_bun(directory: &Path, bun: &Path, arguments: &[&str]) -> Output {
    run_package_runner(directory, "bun", bun, arguments)
}

pub(super) fn start_package_runner(
    directory: &Path,
    command: &str,
    executable: &Path,
    arguments: &[&str],
) -> (Child, u16) {
    let mut child = ragavan_command(directory)
        .arg("__run")
        .arg(command)
        .arg(executable)
        .arg("0")
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
        if argument == ARGUMENTS_END {
            break;
        }
        delivered_arguments.push(argument);
    }
    assert_eq!(
        &delivered_arguments[..arguments.len()],
        arguments,
        "the original runner arguments should be preserved"
    );
    let port = port_from_arguments(&delivered_arguments);

    (child, port)
}

pub(super) fn stop_runner(mut child: Child) {
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

pub(super) fn development_port(directory: &Path) -> u16 {
    development_port_for(directory, "bun", &["dev"])
}

pub(super) fn development_port_for(directory: &Path, command: &str, arguments: &[&str]) -> u16 {
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
        .expect("the development server should receive a port")
        + 1;
    arguments
        .get(port_index)
        .expect("the managed port should have a value")
        .parse()
        .expect("the managed port should be numeric")
}
