use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

pub const ENABLED: &str = "Ragavan is enabled for this repository.\n";

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

pub struct TestRepository {
    directory: TempDirectory,
    path: PathBuf,
}

impl TestRepository {
    pub fn new() -> Self {
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
        assert_success(&git(&path, &["commit", "--allow-empty", "-m", "initial"]));

        Self { directory, path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn add_worktree(&self, name: &str) -> PathBuf {
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

pub struct TempDirectory(PathBuf);

impl TempDirectory {
    pub fn new() -> Self {
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

    pub fn path(&self) -> &Path {
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

pub fn ragavan(directory: &Path, arguments: &[&str]) -> Output {
    ragavan_command(directory)
        .args(arguments)
        .output()
        .expect("Ragavan should start")
}

pub fn ragavan_command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ragavan"));
    command
        .current_dir(directory)
        .env(state_home_variable(), test_state_home(directory));
    command
}

pub fn state_home_variable() -> &'static str {
    if cfg!(windows) {
        "LOCALAPPDATA"
    } else {
        "XDG_STATE_HOME"
    }
}

pub fn test_state_home(directory: &Path) -> PathBuf {
    directory
        .ancestors()
        .find(|ancestor| {
            ancestor
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("ragavan-test-"))
        })
        .unwrap_or(directory)
        .join(".ragavan-state")
}

pub fn git(directory: &Path, arguments: &[&str]) -> Output {
    Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("Git should start")
}

pub fn assert_stdout(output: Output, expected: &str) {
    assert_success(&output);
    assert_eq!(stdout(&output), expected, "{output:?}");
    assert_eq!(stderr(&output), "", "{output:?}");
}

pub fn assert_success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
