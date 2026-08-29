#![forbid(unsafe_code)]

use ragavan_core::{Enrollment, RepositoryId, WorktreeIdentity};
use ragavan_diagnostics::{Detail, Diagnostic};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io,
    path::{Component, Path, PathBuf},
    process::{Command, ExitStatus, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const ENROLLMENT_KEY: &str = "ragavan.enabled";
const REPOSITORY_ID_KEY: &str = "ragavan.repositoryId";
const REPOSITORY_LOCK_FILE: &str = "ragavan.lock";
const GIT_CONFIG_GET_MISSING: i32 = 1;
const GIT_CONFIG_UNSET_MISSING: i32 = 5;
static NEXT_REPOSITORY_ID: AtomicU64 = AtomicU64::new(0);

/// An exclusively held Git repository awaiting the final enable transition.
pub struct EnableRepository {
    common_directory: PathBuf,
    repository_id: RepositoryId,
    operation_lock: File,
}

impl EnableRepository {
    /// Return the identity that must be registered before enabling completes.
    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    /// Return the Git common directory that must be registered.
    pub fn common_directory(&self) -> &Path {
        &self.common_directory
    }

    /// Mark the repository enabled after its global registration succeeds.
    pub fn complete(self) -> Result<Enrollment, Error> {
        const OPERATION: &str = "enable Ragavan for the repository";
        let Self {
            common_directory,
            repository_id: _,
            operation_lock: _operation_lock,
        } = self;
        replace_local_config_at(OPERATION, &common_directory, ENROLLMENT_KEY, "true")?;
        Ok(Enrollment::Enabled)
    }
}

/// Validate a repository and ensure it has an identity without marking it enabled.
pub fn begin_enable(directory: &Path) -> Result<EnableRepository, Error> {
    const OPERATION: &str = "enable Ragavan for the repository";
    let context = repository_context_at(directory, false)?.ok_or(Error::WorktreeRequired {
        operation: OPERATION,
    })?;
    let operation_lock = lock_repository(&context.common_dir)?;
    let repository_id = ensure_repository_id(&context.common_dir)?;
    Ok(EnableRepository {
        common_directory: context.common_dir,
        repository_id,
        operation_lock,
    })
}

/// Read the repository enrollment stored by Git.
pub fn status(directory: &Path) -> Result<Enrollment, Error> {
    const OPERATION: &str = "read the repository enrollment";
    let output = git_at(
        OPERATION,
        directory,
        &["config", "--local", "--bool", "--get", ENROLLMENT_KEY],
    )?;
    enrollment_from_output(OPERATION, output)
}

/// An exclusively held disabled repository retaining its identity until unregistration succeeds.
pub struct DisableRepository {
    common_directory: PathBuf,
    repository_id: Option<RepositoryId>,
    operation_lock: File,
}

impl DisableRepository {
    /// Return the identity to unregister, when it was valid.
    pub fn repository_id(&self) -> Option<&RepositoryId> {
        self.repository_id.as_ref()
    }

    /// Return the Git common directory paired with the registration.
    pub fn common_directory(&self) -> &Path {
        &self.common_directory
    }

    /// Remove the retained repository identity after global unregistration succeeds.
    pub fn complete(self) -> Result<Enrollment, Error> {
        const OPERATION: &str = "disable Ragavan for the repository";
        let Self {
            common_directory,
            repository_id: _,
            operation_lock: _operation_lock,
        } = self;
        unset_local_config_at(OPERATION, &common_directory, REPOSITORY_ID_KEY)?;
        Ok(Enrollment::Disabled)
    }
}

/// Mark a repository disabled while retaining the identity needed for unregistration.
pub fn begin_disable(directory: &Path) -> Result<DisableRepository, Error> {
    const OPERATION: &str = "disable Ragavan for the repository";
    let context = repository_context_at(directory, false)?.ok_or(Error::WorktreeRequired {
        operation: OPERATION,
    })?;
    let operation_lock = lock_repository(&context.common_dir)?;
    let repository_id = match repository_id_at(OPERATION, &context.common_dir) {
        Ok(value) => value.and_then(|value| RepositoryId::new(value).ok()),
        Err(Error::InvalidRepositoryId) => None,
        Err(error) => return Err(error),
    };
    unset_local_config_at(OPERATION, &context.common_dir, ENROLLMENT_KEY)?;
    Ok(DisableRepository {
        common_directory: context.common_dir,
        repository_id,
        operation_lock,
    })
}

/// Begin a managed-command decision for the enabled worktree containing `directory`.
///
/// The returned value exclusively owns repository lifecycle changes until it is dropped.
pub fn begin_managed_command(directory: &Path) -> Result<Option<ManagedWorktree>, Error> {
    let Some(context) = repository_context_at(directory, true)? else {
        return Ok(None);
    };
    if matches!(status(directory)?, Enrollment::Disabled) {
        return Ok(None);
    }
    let operation_lock = lock_repository(&context.common_dir)?;
    if matches!(status(directory)?, Enrollment::Disabled) {
        return Ok(None);
    }

    let identity = identity_for(&context)?;
    Ok(Some(ManagedWorktree {
        context,
        identity,
        _operation_lock: operation_lock,
    }))
}

/// Return tracked and unignored untracked files with the requested basename.
pub fn source_files_named(root: &Path, file_name: &str) -> Result<Vec<PathBuf>, Error> {
    const OPERATION: &str = "list repository source files";
    if file_name.is_empty()
        || !file_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(Error::InvalidSourceFileName(file_name.to_owned()));
    }

    let pathspec = format!(":(top,glob)**/{file_name}");
    let output = git_at(
        OPERATION,
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            &pathspec,
        ],
    )?;
    if !output.status.success() {
        return Err(Error::git(OPERATION, output));
    }

    let stdout = std::str::from_utf8(&output.stdout).map_err(|source| Error::NonUtf8GitOutput {
        operation: OPERATION,
        source,
    })?;
    if !stdout.is_empty() && !stdout.ends_with('\0') {
        return Err(Error::UnexpectedGitOutput {
            operation: OPERATION,
            output: stdout.to_owned(),
        });
    }

    stdout
        .split_terminator('\0')
        .map(|file| {
            let path = Path::new(file);
            if !path
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
                || path.file_name().is_none_or(|name| name != file_name)
            {
                return Err(Error::UnexpectedGitOutput {
                    operation: OPERATION,
                    output: file.to_owned(),
                });
            }
            Ok(path.to_owned())
        })
        .collect()
}

/// An enabled worktree held for one managed-command decision.
pub struct ManagedWorktree {
    context: RepositoryContext,
    identity: WorktreeIdentity,
    _operation_lock: File,
}

impl ManagedWorktree {
    /// Return the worktree root.
    pub fn root(&self) -> &Path {
        &self.context.root
    }

    /// Return the repository and worktree identity.
    pub fn identity(&self) -> &WorktreeIdentity {
        &self.identity
    }

    /// Return the repository's Git common directory.
    pub fn common_directory(&self) -> &Path {
        &self.context.common_dir
    }
}

fn lock_repository(common_directory: &Path) -> Result<File, Error> {
    let path = common_directory.join(REPOSITORY_LOCK_FILE);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| Error::RepositoryLock {
            path: path.clone(),
            source,
        })?;
    lock.lock()
        .map_err(|source| Error::RepositoryLock { path, source })?;
    Ok(lock)
}

fn identity_for(context: &RepositoryContext) -> Result<WorktreeIdentity, Error> {
    let repository_id =
        repository_id_at("read the Ragavan repository identity", &context.common_dir)?
            .ok_or(Error::MissingRepositoryId)
            .and_then(parse_repository_id)?;
    let worktree_id = worktree_id(context)?;

    WorktreeIdentity::new(repository_id, worktree_id).map_err(|_| Error::InvalidWorktreeId)
}

fn worktree_id(context: &RepositoryContext) -> Result<String, Error> {
    if context.git_dir == context.common_dir {
        return Ok("main".to_owned());
    }

    let worktree_id = context
        .git_dir
        .strip_prefix(&context.common_dir)
        .map_err(|_| Error::UnexpectedRepositoryLayout {
            common_dir: context.common_dir.clone(),
            git_dir: context.git_dir.clone(),
        })?
        .to_string_lossy()
        .replace('\\', "/");
    if worktree_id.is_empty() {
        Err(Error::InvalidWorktreeId)
    } else {
        Ok(worktree_id)
    }
}

fn ensure_repository_id(common_directory: &Path) -> Result<RepositoryId, Error> {
    const OPERATION: &str = "enable Ragavan for the repository";
    if let Some(repository_id) = repository_id_at(OPERATION, common_directory)? {
        return parse_repository_id(repository_id);
    }

    let repository_id = new_repository_id();
    replace_local_config_at(
        OPERATION,
        common_directory,
        REPOSITORY_ID_KEY,
        repository_id.as_str(),
    )?;
    Ok(repository_id)
}

fn parse_repository_id(value: String) -> Result<RepositoryId, Error> {
    RepositoryId::new(value).map_err(|_| Error::InvalidRepositoryId)
}

fn new_repository_id() -> RepositoryId {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_REPOSITORY_ID.fetch_add(1, Ordering::Relaxed);
    let process_id = std::process::id();

    RepositoryId::new(format!("{timestamp:032x}{process_id:08x}{sequence:016x}"))
        .expect("generated repository identities must satisfy the domain format")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepositoryIdentity {
    Missing,
    Invalid,
    Valid(RepositoryId),
}

impl RepositoryIdentity {
    pub fn repository_id(&self) -> Option<&RepositoryId> {
        match self {
            Self::Valid(repository_id) => Some(repository_id),
            Self::Missing | Self::Invalid => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// The stable Git location shared by every worktree in one repository.
pub struct RepositoryLocation {
    common_directory: PathBuf,
}

impl RepositoryLocation {
    /// Return the Git common directory.
    pub fn common_directory(&self) -> &Path {
        &self.common_directory
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryInspection {
    enrollment: Enrollment,
    identity: RepositoryIdentity,
    worktrees: Vec<WorktreeInspection>,
}

impl RepositoryInspection {
    pub fn enrollment(&self) -> Enrollment {
        self.enrollment
    }

    pub fn identity(&self) -> &RepositoryIdentity {
        &self.identity
    }

    pub fn worktrees(&self) -> &[WorktreeInspection] {
        &self.worktrees
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorktreeInspection {
    Available { id: String, path: PathBuf },
    Unavailable { path: PathBuf },
}

impl WorktreeInspection {
    pub fn path(&self) -> &Path {
        match self {
            Self::Available { path, .. } | Self::Unavailable { path } => path,
        }
    }
}

/// Identify the Git repository containing a directory without changing state.
pub fn locate_repository(directory: &Path) -> Result<Option<RepositoryLocation>, Error> {
    let Some(context) = repository_context_at(directory, false)? else {
        return Ok(None);
    };
    Ok(Some(RepositoryLocation {
        common_directory: context.common_dir,
    }))
}

/// Inspect one registered repository and all worktrees Git can currently describe.
pub fn inspect_repository(common_directory: &Path) -> Result<Option<RepositoryInspection>, Error> {
    match fs::metadata(common_directory) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(Error::InvalidCommonDirectory {
                path: common_directory.to_owned(),
            });
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::InspectCommonDirectory {
                path: common_directory.to_owned(),
                source,
            });
        }
    }

    let enrollment = enrollment_at(common_directory)?;
    let identity = match repository_id_at(
        "read a registered Ragavan repository identity",
        common_directory,
    ) {
        Ok(value) => configured_identity(value),
        Err(Error::InvalidRepositoryId) => RepositoryIdentity::Invalid,
        Err(error) => return Err(error),
    };
    let worktrees = list_worktrees(common_directory)?;

    Ok(Some(RepositoryInspection {
        enrollment,
        identity,
        worktrees,
    }))
}

fn configured_identity(value: Option<String>) -> RepositoryIdentity {
    match value {
        None => RepositoryIdentity::Missing,
        Some(value) => {
            RepositoryId::new(value).map_or(RepositoryIdentity::Invalid, RepositoryIdentity::Valid)
        }
    }
}

fn enrollment_at(common_directory: &Path) -> Result<Enrollment, Error> {
    const OPERATION: &str = "read a registered repository enrollment";
    let output = git_in_common_directory(
        OPERATION,
        common_directory,
        &["config", "--local", "--bool", "--get", ENROLLMENT_KEY],
    )?;
    enrollment_from_output(OPERATION, output)
}

fn enrollment_from_output(operation: &'static str, output: Output) -> Result<Enrollment, Error> {
    if output.status.success() {
        return match output.stdout.trim_ascii() {
            b"true" => Ok(Enrollment::Enabled),
            b"false" => Ok(Enrollment::Disabled),
            _ => Err(Error::UnexpectedGitOutput {
                operation,
                output: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            }),
        };
    }
    if output.status.code() == Some(GIT_CONFIG_GET_MISSING) {
        return Ok(Enrollment::Disabled);
    }
    Err(Error::git(operation, output))
}

fn repository_id_at(
    operation: &'static str,
    common_directory: &Path,
) -> Result<Option<String>, Error> {
    let output = git_in_common_directory(
        operation,
        common_directory,
        &[
            "config",
            "--local",
            "--null",
            "--get-all",
            REPOSITORY_ID_KEY,
        ],
    )?;
    if output.status.success() {
        let value = output
            .stdout
            .strip_suffix(b"\0")
            .ok_or(Error::InvalidRepositoryId)?;
        if value.contains(&0) {
            return Err(Error::InvalidRepositoryId);
        }
        let value = std::str::from_utf8(value).map_err(|_| Error::InvalidRepositoryId)?;
        return Ok(Some(value.to_owned()));
    }
    if output.status.code() == Some(GIT_CONFIG_GET_MISSING) {
        return Ok(None);
    }
    Err(Error::git(operation, output))
}

fn replace_local_config_at(
    operation: &'static str,
    common_directory: &Path,
    key: &str,
    value: &str,
) -> Result<(), Error> {
    let output = git_in_common_directory(
        operation,
        common_directory,
        &["config", "--local", "--replace-all", key, value],
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::git(operation, output))
    }
}

fn unset_local_config_at(
    operation: &'static str,
    common_directory: &Path,
    key: &str,
) -> Result<(), Error> {
    let output = git_in_common_directory(
        operation,
        common_directory,
        &["config", "--local", "--unset-all", key],
    )?;
    if output.status.success() || output.status.code() == Some(GIT_CONFIG_UNSET_MISSING) {
        Ok(())
    } else {
        Err(Error::git(operation, output))
    }
}

fn list_worktrees(common_directory: &Path) -> Result<Vec<WorktreeInspection>, Error> {
    const OPERATION: &str = "list registered repository worktrees";
    let output = git_in_common_directory(
        OPERATION,
        common_directory,
        &["worktree", "list", "--porcelain", "-z"],
    )?;
    if !output.status.success() {
        return Err(Error::git(OPERATION, output));
    }
    let stdout = std::str::from_utf8(&output.stdout).map_err(|source| Error::NonUtf8GitOutput {
        operation: OPERATION,
        source,
    })?;
    if !stdout.is_empty() && !stdout.ends_with("\0\0") {
        return Err(Error::UnexpectedGitOutput {
            operation: OPERATION,
            output: stdout.to_owned(),
        });
    }

    let mut worktrees = Vec::new();
    for record in stdout.split("\0\0").filter(|record| !record.is_empty()) {
        let mut fields = record.split('\0');
        let Some(path) = fields
            .next()
            .and_then(|field| field.strip_prefix("worktree "))
        else {
            return Err(Error::UnexpectedGitOutput {
                operation: OPERATION,
                output: record.to_owned(),
            });
        };
        let path = PathBuf::from(path);
        let prunable = fields.any(|field| field == "prunable" || field.starts_with("prunable "));
        let worktree = if prunable {
            WorktreeInspection::Unavailable { path }
        } else {
            match repository_context_at(&path, true)? {
                Some(context) => WorktreeInspection::Available {
                    id: worktree_id(&context)?,
                    path,
                },
                None => WorktreeInspection::Unavailable { path },
            }
        };
        worktrees.push(worktree);
    }
    worktrees.sort_by(|left, right| left.path().cmp(right.path()));
    Ok(worktrees)
}

struct RepositoryContext {
    common_dir: PathBuf,
    git_dir: PathBuf,
    root: PathBuf,
}

fn repository_context_at(
    directory: &Path,
    missing_git_is_absent: bool,
) -> Result<Option<RepositoryContext>, Error> {
    repository_context_in(directory, missing_git_is_absent)
}

fn repository_context_in(
    directory: &Path,
    missing_git_is_absent: bool,
) -> Result<Option<RepositoryContext>, Error> {
    const OPERATION: &str = "identify a Git worktree";
    let output = match Command::new("git")
        .current_dir(directory)
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
        Err(error) if missing_git_is_absent && error.kind() == io::ErrorKind::NotFound => {
            return Ok(None);
        }
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

fn git_at(operation: &'static str, directory: &Path, arguments: &[&str]) -> Result<Output, Error> {
    git_command(operation, directory, arguments)
}

fn git_in_common_directory(
    operation: &'static str,
    common_directory: &Path,
    arguments: &[&str],
) -> Result<Output, Error> {
    Command::new("git")
        .arg("--git-dir")
        .arg(common_directory)
        .args(arguments)
        .output()
        .map_err(|source| Error::StartGit { operation, source })
}

fn git_command(
    operation: &'static str,
    directory: &Path,
    arguments: &[&str],
) -> Result<Output, Error> {
    Command::new("git")
        .current_dir(directory)
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
    InvalidSourceFileName(String),
    InvalidRepositoryId,
    InvalidWorktreeId,
    MissingRepositoryId,
    UnexpectedRepositoryLayout {
        common_dir: PathBuf,
        git_dir: PathBuf,
    },
    WorktreeRequired {
        operation: &'static str,
    },
    InvalidCommonDirectory {
        path: PathBuf,
    },
    InspectCommonDirectory {
        path: PathBuf,
        source: io::Error,
    },
    RepositoryLock {
        path: PathBuf,
        source: io::Error,
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
            Self::InvalidSourceFileName(file_name) => write!(
                formatter,
                "source filename `{file_name}` must contain only letters, numbers, dots, hyphens, or underscores"
            ),
            Self::InvalidRepositoryId => {
                formatter.write_str("the Ragavan repository identity is malformed")
            }
            Self::InvalidWorktreeId => {
                formatter.write_str("Git returned an empty worktree identity")
            }
            Self::MissingRepositoryId => {
                formatter.write_str("this repository has no Ragavan repository identity")
            }
            Self::UnexpectedRepositoryLayout {
                common_dir,
                git_dir,
            } => write!(
                formatter,
                "Git worktree directory {} is outside common directory {}",
                git_dir.display(),
                common_dir.display()
            ),
            Self::WorktreeRequired { operation } => {
                write!(
                    formatter,
                    "could not {operation}: the selected directory is not in a Git worktree"
                )
            }
            Self::InvalidCommonDirectory { path } => write!(
                formatter,
                "registered Git common directory {} is not a directory",
                path.display()
            ),
            Self::InspectCommonDirectory { path, source } => write!(
                formatter,
                "could not inspect registered Git common directory {}: {source}",
                path.display()
            ),
            Self::RepositoryLock { path, source } => write!(
                formatter,
                "could not coordinate Ragavan repository operations at {}: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StartGit { source, .. } => Some(source),
            Self::NonUtf8GitOutput { source, .. } => Some(source),
            Self::InspectCommonDirectory { source, .. } => Some(source),
            Self::RepositoryLock { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl Diagnostic for Error {
    fn code(&self) -> &'static str {
        match self {
            Self::StartGit { .. } => "git.start",
            Self::Git { .. } => "git.command",
            Self::UnexpectedGitOutput { .. } => "git.output.unexpected",
            Self::NonUtf8GitOutput { .. } => "git.output.non_utf8",
            Self::InvalidSourceFileName(_) => "git.source_filename.invalid",
            Self::InvalidRepositoryId => "git.repository_identity.invalid",
            Self::InvalidWorktreeId => "git.worktree_identity.invalid",
            Self::MissingRepositoryId => "git.repository_identity.missing",
            Self::UnexpectedRepositoryLayout { .. } => "git.worktree_layout.unexpected",
            Self::WorktreeRequired { .. } => "git.worktree.required",
            Self::InvalidCommonDirectory { .. } => "git.common_directory.invalid",
            Self::InspectCommonDirectory { .. } => "git.common_directory.inspect",
            Self::RepositoryLock { .. } => "git.repository.lock",
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            Self::InvalidRepositoryId => {
                Some("disable and re-enable Ragavan for this repository".to_owned())
            }
            Self::MissingRepositoryId => Some("enable Ragavan for this repository".to_owned()),
            Self::WorktreeRequired { .. } => {
                Some("select a directory in a non-bare Git worktree".to_owned())
            }
            _ => None,
        }
    }

    fn details(&self) -> Vec<Detail> {
        match self {
            Self::StartGit { operation, .. } | Self::NonUtf8GitOutput { operation, .. } => {
                vec![Detail::text("operation", *operation)]
            }
            Self::Git {
                operation,
                status,
                detail,
            } => vec![
                Detail::text("operation", *operation),
                Detail::text("status", status.to_string()),
                Detail::text("output", detail),
            ],
            Self::UnexpectedGitOutput { operation, output } => vec![
                Detail::text("operation", *operation),
                Detail::text("output", output),
            ],
            Self::InvalidSourceFileName(file_name) => {
                vec![Detail::text("filename", file_name)]
            }
            Self::UnexpectedRepositoryLayout {
                common_dir,
                git_dir,
            } => vec![
                Detail::text("common_directory", common_dir.display().to_string()),
                Detail::text("worktree_directory", git_dir.display().to_string()),
            ],
            Self::WorktreeRequired { operation } => {
                vec![Detail::text("operation", *operation)]
            }
            Self::InvalidCommonDirectory { path }
            | Self::InspectCommonDirectory { path, .. }
            | Self::RepositoryLock { path, .. } => {
                vec![Detail::text("path", path.display().to_string())]
            }
            Self::InvalidRepositoryId | Self::InvalidWorktreeId | Self::MissingRepositoryId => {
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Enrollment, REPOSITORY_LOCK_FILE, RepositoryIdentity, begin_disable, begin_enable,
        inspect_repository, status,
    };
    use std::{
        fs::{self, OpenOptions, TryLockError},
        io::{self, Write},
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn a_repository_transition_owns_exclusive_mutation_until_it_finishes() {
        let repository = TestRepository::new();
        let transition = begin_enable(repository.path())
            .expect("the repository should begin its enable transition");
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(transition.common_directory().join(REPOSITORY_LOCK_FILE))
            .expect("the repository lock should exist");

        assert!(matches!(
            contender.try_lock(),
            Err(TryLockError::WouldBlock)
        ));
        assert_eq!(
            transition.complete().expect("enabling should complete"),
            Enrollment::Enabled
        );
        contender
            .try_lock()
            .expect("finishing the transition should release the repository");
    }

    #[test]
    fn a_malformed_repository_identity_remains_inspectable_and_disableable() {
        let repository = TestRepository::new();
        let common_directory = repository.path().join(".git");
        OpenOptions::new()
            .append(true)
            .open(common_directory.join("config"))
            .expect("the Git config should open")
            .write_all(b"\n[ragavan]\n\trepositoryId = \xff\n\tenabled = true\n")
            .expect("the malformed repository identity should be written");

        assert_invalid_identity_can_be_disabled(&repository);
    }

    #[test]
    fn multiple_repository_identities_remain_inspectable_and_disableable() {
        let repository = TestRepository::new();
        OpenOptions::new()
            .append(true)
            .open(repository.path().join(".git").join("config"))
            .expect("the Git config should open")
            .write_all(
                b"\n[ragavan]\n\trepositoryId = first\n\trepositoryId = second\n\tenabled = true\n",
            )
            .expect("the duplicate repository identities should be written");

        assert_invalid_identity_can_be_disabled(&repository);
    }

    fn assert_invalid_identity_can_be_disabled(repository: &TestRepository) {
        let inspection = inspect_repository(&repository.path().join(".git"))
            .expect("the repository should remain inspectable")
            .expect("the repository should exist");
        assert_eq!(inspection.identity(), &RepositoryIdentity::Invalid);

        let transition = begin_disable(repository.path())
            .expect("a malformed identity should not prevent disabling");
        assert_eq!(transition.repository_id(), None);
        assert_eq!(
            transition.complete().expect("disabling should complete"),
            Enrollment::Disabled
        );
        assert_eq!(
            status(repository.path()).expect("enrollment should remain readable"),
            Enrollment::Disabled
        );
    }

    struct TestRepository {
        container: PathBuf,
        path: PathBuf,
    }

    impl TestRepository {
        fn new() -> Self {
            for _ in 0..100 {
                let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let container = std::env::temp_dir().join(format!(
                    "ragavan-git-test-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&container) {
                    Ok(()) => {
                        let path = container.join("repository");
                        let output = Command::new("git")
                            .args(["init", "--initial-branch=main"])
                            .arg(&path)
                            .output()
                            .expect("Git should start");
                        assert!(output.status.success(), "{output:?}");
                        return Self { container, path };
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        panic!("could not create test directory {container:?}: {error}")
                    }
                }
            }

            panic!("could not allocate a unique test directory")
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestRepository {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.container) {
                eprintln!(
                    "could not remove test directory {:?}: {error}",
                    self.container
                );
            }
        }
    }
}
