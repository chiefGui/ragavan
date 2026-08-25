#![forbid(unsafe_code)]

use ragavan_core::{Enrollment, IdentityError, WorktreeIdentity};
use std::{
    fmt, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const ENROLLMENT_KEY: &str = "ragavan.enabled";
const REPOSITORY_ID_KEY: &str = "ragavan.repositoryId";
const GIT_CONFIG_GET_MISSING: i32 = 1;
const GIT_CONFIG_UNSET_MISSING: i32 = 5;
static NEXT_REPOSITORY_ID: AtomicU64 = AtomicU64::new(0);

pub fn enable() -> Result<Enrollment, Error> {
    ensure_repository_id()?;

    const OPERATION: &str = "enable Ragavan for the repository";
    let output = git(
        OPERATION,
        &["config", "--local", "--replace-all", ENROLLMENT_KEY, "true"],
    )?;

    if output.status.success() {
        Ok(Enrollment::Enabled)
    } else {
        Err(Error::git(OPERATION, output))
    }
}

pub fn status() -> Result<Enrollment, Error> {
    const OPERATION: &str = "read the repository enrollment";
    let output = git(
        OPERATION,
        &["config", "--local", "--bool", "--get", ENROLLMENT_KEY],
    )?;

    if output.status.success() {
        return match output.stdout.trim_ascii() {
            b"true" => Ok(Enrollment::Enabled),
            b"false" => Ok(Enrollment::Disabled),
            _ => Err(Error::UnexpectedGitOutput {
                operation: OPERATION,
                output: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            }),
        };
    }
    if output.status.code() == Some(GIT_CONFIG_GET_MISSING) {
        return Ok(Enrollment::Disabled);
    }

    Err(Error::git(OPERATION, output))
}

pub fn disable() -> Result<Enrollment, Error> {
    const OPERATION: &str = "disable Ragavan for the repository";
    unset_local_config(OPERATION, ENROLLMENT_KEY)?;
    unset_local_config(OPERATION, REPOSITORY_ID_KEY)?;
    Ok(Enrollment::Disabled)
}

pub fn enrolled_worktree() -> Result<Option<EnrolledWorktree>, Error> {
    let Some(context) = repository_context()? else {
        return Ok(None);
    };
    if matches!(status()?, Enrollment::Disabled) {
        return Ok(None);
    }

    Ok(Some(EnrolledWorktree { context }))
}

pub struct EnrolledWorktree {
    context: RepositoryContext,
}

impl EnrolledWorktree {
    pub fn root(&self) -> &Path {
        &self.context.root
    }

    pub fn identity(&self) -> Result<WorktreeIdentity, Error> {
        let repository_id =
            local_config("read the Ragavan repository identity", REPOSITORY_ID_KEY)?
                .ok_or(Error::MissingRepositoryId)?;

        let worktree_id = if self.context.git_dir == self.context.common_dir {
            "main".to_owned()
        } else {
            self.context
                .git_dir
                .strip_prefix(&self.context.common_dir)
                .map_err(|_| Error::UnexpectedRepositoryLayout {
                    common_dir: self.context.common_dir.clone(),
                    git_dir: self.context.git_dir.clone(),
                })?
                .to_string_lossy()
                .replace('\\', "/")
        };

        WorktreeIdentity::new(repository_id, worktree_id).map_err(|error| match error {
            IdentityError::EmptyRepository => Error::InvalidRepositoryId,
            IdentityError::EmptyWorktree => Error::InvalidWorktreeId,
        })
    }
}

fn ensure_repository_id() -> Result<(), Error> {
    const OPERATION: &str = "enable Ragavan for the repository";
    if let Some(repository_id) = local_config(OPERATION, REPOSITORY_ID_KEY)? {
        if repository_id.is_empty() {
            return Err(Error::InvalidRepositoryId);
        }
        return Ok(());
    }

    let repository_id = new_repository_id();
    let output = git(
        OPERATION,
        &[
            "config",
            "--local",
            "--replace-all",
            REPOSITORY_ID_KEY,
            &repository_id,
        ],
    )?;

    if output.status.success() {
        Ok(())
    } else {
        Err(Error::git(OPERATION, output))
    }
}

fn new_repository_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_REPOSITORY_ID.fetch_add(1, Ordering::Relaxed);
    let process_id = std::process::id();

    format!("{timestamp:032x}{process_id:08x}{sequence:016x}")
}

fn local_config(operation: &'static str, key: &str) -> Result<Option<String>, Error> {
    let output = git(operation, &["config", "--local", "--get", key])?;

    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ));
    }
    if output.status.code() == Some(GIT_CONFIG_GET_MISSING) {
        return Ok(None);
    }

    Err(Error::git(operation, output))
}

fn unset_local_config(operation: &'static str, key: &str) -> Result<(), Error> {
    let output = git(operation, &["config", "--local", "--unset-all", key])?;

    if output.status.success() || output.status.code() == Some(GIT_CONFIG_UNSET_MISSING) {
        Ok(())
    } else {
        Err(Error::git(operation, output))
    }
}

struct RepositoryContext {
    common_dir: PathBuf,
    git_dir: PathBuf,
    root: PathBuf,
}

fn repository_context() -> Result<Option<RepositoryContext>, Error> {
    const OPERATION: &str = "identify the current Git worktree";
    let output = match Command::new("git")
        .args([
            "rev-parse",
            "--is-inside-work-tree",
            "--path-format=absolute",
            "--git-common-dir",
            "--git-dir",
            "--show-toplevel",
        ])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::StartGit {
                operation: OPERATION,
                source,
            });
        }
    };

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = std::str::from_utf8(&output.stdout).map_err(|source| Error::NonUtf8GitOutput {
        operation: OPERATION,
        source,
    })?;
    let mut lines = stdout.lines();
    match (
        lines.next(),
        lines.next(),
        lines.next(),
        lines.next(),
        lines.next(),
    ) {
        (Some("true"), Some(common_dir), Some(git_dir), Some(root), None) => {
            Ok(Some(RepositoryContext {
                common_dir: PathBuf::from(common_dir),
                git_dir: PathBuf::from(git_dir),
                root: PathBuf::from(root),
            }))
        }
        _ => Err(Error::UnexpectedGitOutput {
            operation: OPERATION,
            output: stdout.trim().to_owned(),
        }),
    }
}

fn git(operation: &'static str, arguments: &[&str]) -> Result<Output, Error> {
    Command::new("git")
        .args(arguments)
        .output()
        .map_err(|source| Error::StartGit { operation, source })
}

#[derive(Debug)]
pub enum Error {
    StartGit {
        operation: &'static str,
        source: io::Error,
    },
    Git {
        operation: &'static str,
        status: ExitStatus,
        detail: String,
    },
    UnexpectedGitOutput {
        operation: &'static str,
        output: String,
    },
    NonUtf8GitOutput {
        operation: &'static str,
        source: std::str::Utf8Error,
    },
    InvalidRepositoryId,
    InvalidWorktreeId,
    MissingRepositoryId,
    UnexpectedRepositoryLayout {
        common_dir: PathBuf,
        git_dir: PathBuf,
    },
}

impl Error {
    fn git(operation: &'static str, output: Output) -> Self {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        } else {
            stderr.trim().to_owned()
        };

        Self::Git {
            operation,
            status: output.status,
            detail,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartGit { operation, source } => {
                write!(formatter, "could not start Git to {operation}: {source}")
            }
            Self::Git {
                operation,
                status,
                detail,
            } => {
                write!(formatter, "could not {operation} ({status})")?;
                if !detail.is_empty() {
                    write!(formatter, ": {detail}")?;
                }
                Ok(())
            }
            Self::UnexpectedGitOutput { operation, output } => write!(
                formatter,
                "could not {operation}: Git returned an unexpected value `{output}`"
            ),
            Self::NonUtf8GitOutput { operation, source } => {
                write!(formatter, "could not {operation}: {source}")
            }
            Self::InvalidRepositoryId => formatter.write_str(
                "the Ragavan repository identity is empty; run `ragavan disable` and then `ragavan enable`",
            ),
            Self::InvalidWorktreeId => {
                formatter.write_str("Git returned an empty worktree identity")
            }
            Self::MissingRepositoryId => formatter.write_str(
                "this repository predates worktree isolation; run `ragavan enable` once to finish enrollment",
            ),
            Self::UnexpectedRepositoryLayout {
                common_dir,
                git_dir,
            } => write!(
                formatter,
                "Git worktree directory {} is outside common directory {}",
                git_dir.display(),
                common_dir.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StartGit { source, .. } => Some(source),
            Self::NonUtf8GitOutput { source, .. } => Some(source),
            _ => None,
        }
    }
}
