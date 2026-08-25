use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

const ENABLED: &str = "Ragavan is enabled for this repository.\n";
const DISABLED: &str = "Ragavan is disabled for this repository.\n";

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn enrollment_applies_to_existing_and_future_worktrees() {
    let repository = TestRepository::new();
    assert_stdout(
        git(
            repository.path(),
            &["config", "extensions.worktreeConfig", "true"],
        ),
        "",
    );
    let existing_worktree = repository.add_worktree("existing");

    assert_stdout(ragavan(&existing_worktree, &["status"]), DISABLED);
    assert_stdout(ragavan(&existing_worktree, &["enable"]), ENABLED);
    assert_stdout(ragavan(repository.path(), &["status"]), ENABLED);

    let future_worktree = repository.add_worktree("future");
    assert_stdout(ragavan(&future_worktree, &["status"]), ENABLED);
    assert_stdout(ragavan(&future_worktree, &["disable"]), DISABLED);
    assert_stdout(ragavan(&existing_worktree, &["status"]), DISABLED);

    for worktree in [repository.path(), &existing_worktree, &future_worktree] {
        assert_stdout(git(worktree, &["status", "--porcelain"]), "");
    }
}

#[test]
fn enrollment_changes_are_idempotent_and_reversible() {
    let repository = TestRepository::new();

    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);
    assert_stdout(ragavan(repository.path(), &["enable"]), ENABLED);
    assert_stdout(ragavan(repository.path(), &["disable"]), DISABLED);
    assert_stdout(ragavan(repository.path(), &["disable"]), DISABLED);
    assert_stdout(ragavan(repository.path(), &["status"]), DISABLED);
}

#[test]
fn enrollment_does_not_apply_to_other_repositories() {
    let enrolled_repository = TestRepository::new();
    let other_repository = TestRepository::new();

    assert_stdout(ragavan(enrolled_repository.path(), &["enable"]), ENABLED);
    assert_stdout(ragavan(other_repository.path(), &["status"]), DISABLED);
}

#[test]
fn enrollment_requires_a_git_repository() {
    let directory = TempDirectory::new();
    let output = ragavan(directory.path(), &["enable"]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        stderr(&output).contains("could not enable Ragavan for the repository"),
        "{output:?}"
    );
}

#[test]
fn missing_git_is_reported() {
    let directory = TempDirectory::new();
    let output = Command::new(env!("CARGO_BIN_EXE_ragavan"))
        .current_dir(directory.path())
        .arg("status")
        .env("PATH", "")
        .output()
        .expect("Ragavan should start");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        stderr(&output).contains("could not start Git to read the repository enrollment"),
        "{output:?}"
    );
}

#[test]
fn unknown_commands_are_rejected_before_repository_access() {
    let directory = TempDirectory::new();
    let output = ragavan(directory.path(), &["launch"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(stderr(&output).contains("unknown command `launch`"));
    assert!(stderr(&output).contains("Usage: ragavan <COMMAND>"));
}

#[test]
fn no_command_prints_help_before_repository_access() {
    let directory = TempDirectory::new();
    let output = ragavan(directory.path(), &[]);

    assert_success(&output);
    assert!(stdout(&output).contains("Usage: ragavan <COMMAND>"));
    assert!(stdout(&output).contains("enable"));
    assert_eq!(stderr(&output), "");
}

struct TestRepository {
    directory: TempDirectory,
    path: PathBuf,
}

impl TestRepository {
    fn new() -> Self {
        let directory = TempDirectory::new();
        let path = directory.path().join("repository");

        let output = Command::new("git")
            .args(["init", "--initial-branch=main"])
            .arg(&path)
            .output()
            .expect("Git should start");
        assert_success(&output);

        assert_stdout(git(&path, &["config", "user.name", "Ragavan Tests"]), "");
        assert_stdout(
            git(
                &path,
                &["config", "user.email", "ragavan-tests@example.invalid"],
            ),
            "",
        );
        let output = git(&path, &["commit", "--allow-empty", "-m", "initial"]);
        assert_success(&output);

        Self { directory, path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn add_worktree(&self, name: &str) -> PathBuf {
        let path = self.directory.path().join(name);
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["worktree", "add", "--detach"])
            .arg(&path)
            .output()
            .expect("Git should start");
        assert_success(&output);
        path
    }
}

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        for _ in 0..100 {
            let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path =
                env::temp_dir().join(format!("ragavan-test-{}-{sequence}", std::process::id()));

            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("could not create test directory {path:?}: {error}"),
            }
        }

        panic!("could not allocate a unique test directory");
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("could not remove test directory {:?}: {error}", self.0);
        }
    }
}

fn ragavan(directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ragavan"))
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("Ragavan should start")
}

fn git(directory: &Path, arguments: &[&str]) -> Output {
    Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("Git should start")
}

fn assert_stdout(output: Output, expected: &str) {
    assert_success(&output);
    assert_eq!(stdout(&output), expected, "{output:?}");
    assert_eq!(stderr(&output), "", "{output:?}");
}

fn assert_success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
