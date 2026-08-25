#![forbid(unsafe_code)]

use std::{
    env,
    ffi::OsString,
    fmt, fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitCode, ExitStatus, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const ENROLLMENT_KEY: &str = "ragavan.enabled";
const REPOSITORY_ID_KEY: &str = "ragavan.repositoryId";
const GIT_CONFIG_GET_MISSING: i32 = 1;
const GIT_CONFIG_UNSET_MISSING: i32 = 5;
const PASSTHROUGH_EXIT_CODE: u8 = 10;
const PORT_RANGE_START: u16 = 10_000;
const PORT_RANGE_SIZE: u64 = 20_000;
static NEXT_REPOSITORY_ID: AtomicU64 = AtomicU64::new(0);
const USAGE: &str = "\
Usage: ragavan <COMMAND>

Commands:
  enable   Enable Ragavan for the current repository
  status   Show whether Ragavan is enabled
  disable  Disable Ragavan for the current repository
  hook     Print shell integration (`ragavan hook powershell`)
  help     Print this help
";

const POWERSHELL_HOOK: &str = r#"$global:__RagavanOriginalBun = Get-Command bun -CommandType Application,ExternalScript -ErrorAction Stop | Select-Object -First 1
$global:__RagavanCommand = Get-Command ragavan -CommandType Application -ErrorAction Stop | Select-Object -First 1

function global:bun {
    $ragavanArguments = & $global:__RagavanCommand __bun-arguments @args
    $ragavanStatus = $LASTEXITCODE

    if ($ragavanStatus -eq 0) {
        & $global:__RagavanOriginalBun @args @ragavanArguments
        return
    }

    if ($ragavanStatus -eq __RAGAVAN_PASSTHROUGH_EXIT_CODE__) {
        & $global:__RagavanOriginalBun @args
        return
    }

    $global:LASTEXITCODE = $ragavanStatus
}
"#;

fn main() -> ExitCode {
    let action = match parse_action() {
        Ok(action) => action,
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match execute(action) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

enum Action {
    Enable,
    Status,
    Disable,
    HookPowerShell,
    BunArguments(Vec<OsString>),
    Help,
}

fn parse_action() -> Result<Action, String> {
    let mut arguments = env::args_os().skip(1);
    let command = arguments.next();

    let action = match command.as_deref() {
        None => Action::Help,
        Some(command) if command == "enable" => Action::Enable,
        Some(command) if command == "status" => Action::Status,
        Some(command) if command == "disable" => Action::Disable,
        Some(command) if command == "hook" => {
            let shell = arguments
                .next()
                .ok_or_else(|| "missing shell; expected `powershell`".to_owned())?;
            if shell != "powershell" {
                return Err(format!(
                    "unsupported shell `{}`; expected `powershell`",
                    shell.to_string_lossy()
                ));
            }
            Action::HookPowerShell
        }
        Some(command) if command == "__bun-arguments" => {
            return Ok(Action::BunArguments(arguments.collect()));
        }
        Some(command) if command == "help" || command == "--help" || command == "-h" => {
            Action::Help
        }
        Some(command) => {
            return Err(format!("unknown command `{}`", command.to_string_lossy()));
        }
    };

    if let Some(argument) = arguments.next() {
        return Err(format!(
            "unexpected argument `{}`",
            argument.to_string_lossy()
        ));
    }

    Ok(action)
}

fn execute(action: Action) -> Result<ExitCode, Failure> {
    match action {
        Action::Help => print!("{USAGE}"),
        Action::Enable => println!("{}", enable()?),
        Action::Status => println!("{}", enrollment()?),
        Action::Disable => println!("{}", disable()?),
        Action::HookPowerShell => print!("{}", powershell_hook()),
        Action::BunArguments(arguments) => {
            let Some(additional_arguments) = bun_arguments(&arguments)? else {
                return Ok(ExitCode::from(PASSTHROUGH_EXIT_CODE));
            };

            for argument in additional_arguments {
                println!("{argument}");
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn powershell_hook() -> String {
    POWERSHELL_HOOK.replace(
        "__RAGAVAN_PASSTHROUGH_EXIT_CODE__",
        &PASSTHROUGH_EXIT_CODE.to_string(),
    )
}

enum Enrollment {
    Enabled,
    Disabled,
}

impl fmt::Display for Enrollment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enabled => formatter.write_str("Ragavan is enabled for this repository."),
            Self::Disabled => formatter.write_str("Ragavan is disabled for this repository."),
        }
    }
}

fn enable() -> Result<Enrollment, Failure> {
    ensure_repository_id()?;

    const OPERATION: &str = "enable Ragavan for the repository";
    let output = git(
        OPERATION,
        &["config", "--local", "--replace-all", ENROLLMENT_KEY, "true"],
    )?;

    if output.status.success() {
        Ok(Enrollment::Enabled)
    } else {
        Err(Failure::git(OPERATION, output))
    }
}

fn enrollment() -> Result<Enrollment, Failure> {
    const OPERATION: &str = "read the repository enrollment";
    let output = git(
        OPERATION,
        &["config", "--local", "--bool", "--get", ENROLLMENT_KEY],
    )?;

    if output.status.success() {
        return match output.stdout.trim_ascii() {
            b"true" => Ok(Enrollment::Enabled),
            b"false" => Ok(Enrollment::Disabled),
            _ => Err(Failure::UnexpectedGitOutput {
                operation: OPERATION,
                output: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            }),
        };
    }
    if output.status.code() == Some(GIT_CONFIG_GET_MISSING) {
        return Ok(Enrollment::Disabled);
    }

    Err(Failure::git(OPERATION, output))
}

fn disable() -> Result<Enrollment, Failure> {
    const OPERATION: &str = "disable Ragavan for the repository";
    unset_local_config(OPERATION, ENROLLMENT_KEY)?;
    unset_local_config(OPERATION, REPOSITORY_ID_KEY)?;
    Ok(Enrollment::Disabled)
}

fn ensure_repository_id() -> Result<(), Failure> {
    const OPERATION: &str = "enable Ragavan for the repository";
    if let Some(repository_id) = local_config(OPERATION, REPOSITORY_ID_KEY)? {
        if repository_id.is_empty() {
            return Err(Failure::InvalidRepositoryId);
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
        Err(Failure::git(OPERATION, output))
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

fn local_config(operation: &'static str, key: &str) -> Result<Option<String>, Failure> {
    let output = git(operation, &["config", "--local", "--get", key])?;

    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ));
    }
    if output.status.code() == Some(GIT_CONFIG_GET_MISSING) {
        return Ok(None);
    }

    Err(Failure::git(operation, output))
}

fn unset_local_config(operation: &'static str, key: &str) -> Result<(), Failure> {
    let output = git(operation, &["config", "--local", "--unset-all", key])?;

    if output.status.success() || output.status.code() == Some(GIT_CONFIG_UNSET_MISSING) {
        Ok(())
    } else {
        Err(Failure::git(operation, output))
    }
}

fn bun_arguments(arguments: &[OsString]) -> Result<Option<[String; 3]>, Failure> {
    let Some(script_index) = bun_dev_script_index(arguments) else {
        return Ok(None);
    };
    let Some(repository) = repository_context()? else {
        return Ok(None);
    };
    if matches!(enrollment()?, Enrollment::Disabled) {
        return Ok(None);
    }

    if arguments[script_index + 1..]
        .iter()
        .filter_map(|argument| argument.to_str())
        .any(is_port_argument)
    {
        return Err(Failure::ExplicitPort);
    }

    let package_path = nearest_package_json(&repository.root)?;
    let package = fs::read(&package_path).map_err(|source| Failure::ReadPackage {
        path: package_path.clone(),
        source,
    })?;
    let package: serde_json::Value =
        serde_json::from_slice(&package).map_err(|source| Failure::ParsePackage {
            path: package_path.clone(),
            source,
        })?;
    let script = package
        .get("scripts")
        .and_then(|scripts| scripts.get("dev"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Failure::MissingDevScript {
            path: package_path.clone(),
        })?;

    if !is_supported_vite_script(script) {
        return Err(Failure::UnsupportedDevScript {
            path: package_path,
            script: script.to_owned(),
        });
    }
    if script.split_ascii_whitespace().any(is_port_argument) {
        return Err(Failure::ExplicitPort);
    }

    let repository_id = local_config("read the Ragavan repository identity", REPOSITORY_ID_KEY)?
        .filter(|repository_id| !repository_id.is_empty())
        .ok_or(Failure::MissingRepositoryId)?;
    let worktree_id = repository.worktree_id()?;
    let port = stable_port(&repository_id, &worktree_id);

    Ok(Some([
        "--port".to_owned(),
        port.to_string(),
        "--strictPort".to_owned(),
    ]))
}

fn bun_dev_script_index(arguments: &[OsString]) -> Option<usize> {
    match arguments {
        [script, ..] if script == "dev" => Some(0),
        [run, script, ..] if run == "run" && script == "dev" => Some(1),
        _ => None,
    }
}

fn is_port_argument(argument: &str) -> bool {
    argument == "--port" || argument.starts_with("--port=")
}

fn is_supported_vite_script(script: &str) -> bool {
    if script
        .chars()
        .any(|character| "\r\n&|;<>".contains(character))
    {
        return false;
    }

    let Some(command) = script.split_ascii_whitespace().next() else {
        return false;
    };
    let command = command.rsplit(['/', '\\']).next().unwrap_or(command);

    matches!(command, "vite" | "vite.cmd" | "vite.exe")
}

struct RepositoryContext {
    common_dir: PathBuf,
    git_dir: PathBuf,
    root: PathBuf,
}

impl RepositoryContext {
    fn worktree_id(&self) -> Result<String, Failure> {
        if self.git_dir == self.common_dir {
            return Ok("main".to_owned());
        }

        let relative = self.git_dir.strip_prefix(&self.common_dir).map_err(|_| {
            Failure::UnexpectedRepositoryLayout {
                common_dir: self.common_dir.clone(),
                git_dir: self.git_dir.clone(),
            }
        })?;

        Ok(relative.to_string_lossy().replace('\\', "/"))
    }
}

fn repository_context() -> Result<Option<RepositoryContext>, Failure> {
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
            return Err(Failure::StartGit {
                operation: OPERATION,
                source,
            });
        }
    };

    if !output.status.success() {
        return Ok(None);
    }

    let stdout =
        std::str::from_utf8(&output.stdout).map_err(|source| Failure::NonUtf8GitOutput {
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
        _ => Err(Failure::UnexpectedGitOutput {
            operation: OPERATION,
            output: stdout.trim().to_owned(),
        }),
    }
}

fn nearest_package_json(worktree_root: &Path) -> Result<PathBuf, Failure> {
    let current_directory = env::current_dir().map_err(Failure::CurrentDirectory)?;

    for directory in current_directory.ancestors() {
        let package_path = directory.join("package.json");
        match fs::metadata(&package_path) {
            Ok(metadata) if metadata.is_file() => return Ok(package_path),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(Failure::ReadPackage {
                    path: package_path,
                    source,
                });
            }
        }

        if directory == worktree_root {
            return Err(Failure::MissingPackage {
                root: worktree_root.to_owned(),
            });
        }
    }

    Err(Failure::CurrentDirectoryOutsideWorktree {
        current_directory,
        root: worktree_root.to_owned(),
    })
}

fn stable_port(repository_id: &str, worktree_id: &str) -> u16 {
    let repository_slot = stable_hash(repository_id) % PORT_RANGE_SIZE;
    let worktree_slot = stable_hash(worktree_id) % PORT_RANGE_SIZE;
    PORT_RANGE_START + ((repository_slot + worktree_slot) % PORT_RANGE_SIZE) as u16
}

fn stable_hash(value: &str) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn git(operation: &'static str, arguments: &[&str]) -> Result<Output, Failure> {
    Command::new("git")
        .args(arguments)
        .output()
        .map_err(|source| Failure::StartGit { operation, source })
}

#[derive(Debug)]
enum Failure {
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
    MissingRepositoryId,
    CurrentDirectory(io::Error),
    CurrentDirectoryOutsideWorktree {
        current_directory: PathBuf,
        root: PathBuf,
    },
    UnexpectedRepositoryLayout {
        common_dir: PathBuf,
        git_dir: PathBuf,
    },
    MissingPackage {
        root: PathBuf,
    },
    ReadPackage {
        path: PathBuf,
        source: io::Error,
    },
    ParsePackage {
        path: PathBuf,
        source: serde_json::Error,
    },
    MissingDevScript {
        path: PathBuf,
    },
    UnsupportedDevScript {
        path: PathBuf,
        script: String,
    },
    ExplicitPort,
}

impl Failure {
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

impl fmt::Display for Failure {
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
            Self::MissingRepositoryId => formatter.write_str(
                "this repository predates worktree isolation; run `ragavan enable` once to finish enrollment",
            ),
            Self::CurrentDirectory(source) => {
                write!(formatter, "could not read the current directory: {source}")
            }
            Self::CurrentDirectoryOutsideWorktree {
                current_directory,
                root,
            } => write!(
                formatter,
                "current directory {} is outside Git worktree {}",
                current_directory.display(),
                root.display()
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
            Self::MissingPackage { root } => write!(
                formatter,
                "could not isolate `bun dev`: no package.json exists between the current directory and {}",
                root.display()
            ),
            Self::ReadPackage { path, source } => {
                write!(formatter, "could not read {}: {source}", path.display())
            }
            Self::ParsePackage { path, source } => {
                write!(formatter, "could not parse {}: {source}", path.display())
            }
            Self::MissingDevScript { path } => write!(
                formatter,
                "could not isolate `bun dev`: {} has no string `scripts.dev`",
                path.display()
            ),
            Self::UnsupportedDevScript { path, script } => write!(
                formatter,
                "could not isolate `bun dev`: {} uses unsupported script `{script}`; this slice recognizes Vite",
                path.display()
            ),
            Self::ExplicitPort => formatter.write_str(
                "could not isolate `bun dev`: an explicit `--port` conflicts with Ragavan's worktree port",
            ),
        }
    }
}
