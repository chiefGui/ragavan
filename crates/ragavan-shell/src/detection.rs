use std::{
    fmt, io,
    path::{Path, PathBuf},
};

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(not(target_os = "linux"))]
use std::process::{Command, ExitStatus, Output};
#[cfg(target_os = "linux")]
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

const OPERATION: &str = "identify the current shell";

pub(super) struct CurrentProcess {
    command: PathBuf,
    executable: PathBuf,
}

impl CurrentProcess {
    pub(super) fn command(&self) -> &Path {
        &self.command
    }

    pub(super) fn into_executable(self) -> PathBuf {
        self.executable
    }
}

#[cfg(windows)]
pub(super) fn current() -> Result<CurrentProcess, Error> {
    let process_id = std::process::id();
    let script = format!(
        "$process = Get-CimInstance Win32_Process -Filter 'ProcessId = {process_id}'; \
         $parent = Get-CimInstance Win32_Process -Filter \"ProcessId = $($process.ParentProcessId)\"; \
         [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); \
         [Console]::Out.Write($parent.ExecutablePath)"
    );
    let helper = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| {
            root.join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .unwrap_or_else(|| PathBuf::from("powershell.exe"));
    let output = Command::new(&helper)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
        .arg(script)
        .output()
        .map_err(|source| Error::StartHelper {
            executable: helper.clone(),
            source,
        })?;
    let executable = successful_output(&helper, output)?;

    let executable = validate_executable(executable)?;
    Ok(CurrentProcess {
        command: executable.clone(),
        executable,
    })
}

#[cfg(target_os = "linux")]
pub(super) fn current() -> Result<CurrentProcess, Error> {
    let stat_path = Path::new("/proc/self/stat");
    let stat = fs::read_to_string(stat_path).map_err(|source| Error::ReadProcess {
        path: stat_path.to_owned(),
        source,
    })?;
    let parent = stat
        .rsplit_once(") ")
        .and_then(|(_, fields)| fields.split_ascii_whitespace().nth(1))
        .filter(|parent| parent.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| Error::MalformedProcess(stat_path.to_owned()))?;
    let process = Path::new("/proc").join(parent);
    let executable_path = process.join("exe");
    let executable = fs::read_link(&executable_path).map_err(|source| Error::ReadProcess {
        path: executable_path,
        source,
    })?;
    let command_path = process.join("cmdline");
    let command = fs::read(&command_path).map_err(|source| Error::ReadProcess {
        path: command_path.clone(),
        source,
    })?;
    let command = command
        .split(|byte| *byte == 0)
        .next()
        .filter(|command| !command.is_empty())
        .ok_or(Error::MalformedProcess(command_path))?;

    Ok(CurrentProcess {
        command: PathBuf::from(OsString::from_vec(command.to_vec())),
        executable,
    })
}

#[cfg(all(not(windows), not(target_os = "linux")))]
pub(super) fn current() -> Result<CurrentProcess, Error> {
    let process_id = std::process::id().to_string();
    let parent = helper_output(
        Command::new("ps")
            .args(["-p", &process_id, "-o", "ppid="])
            .output(),
    )?;
    let parent = parent.trim();
    if parent.is_empty() || !parent.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::UnexpectedOutput(parent.to_owned()));
    }

    let executable = helper_output(
        Command::new("ps")
            .args(["-p", parent, "-o", "comm="])
            .output(),
    )?;
    let executable = validate_executable(executable)?;
    Ok(CurrentProcess {
        command: executable.clone(),
        executable,
    })
}

#[cfg(all(not(windows), not(target_os = "linux")))]
fn helper_output(output: io::Result<Output>) -> Result<String, Error> {
    let helper = Path::new("ps");
    let output = output.map_err(|source| Error::StartHelper {
        executable: helper.to_owned(),
        source,
    })?;
    successful_output(helper, output)
}

#[cfg(not(target_os = "linux"))]
fn successful_output(executable: &Path, output: Output) -> Result<String, Error> {
    if !output.status.success() {
        return Err(Error::Helper {
            executable: executable.to_owned(),
            status: output.status,
            detail: output_detail(&output),
        });
    }

    String::from_utf8(output.stdout).map_err(Error::NonUtf8Output)
}

#[cfg(not(target_os = "linux"))]
fn validate_executable(output: String) -> Result<PathBuf, Error> {
    let output = output.trim();
    if output.is_empty() || output.contains(['\r', '\n', '\0']) {
        return Err(Error::UnexpectedOutput(output.to_owned()));
    }

    Ok(PathBuf::from(output))
}

#[cfg(not(target_os = "linux"))]
fn output_detail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.trim().is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    } else {
        stderr.trim().to_owned()
    }
}

#[derive(Debug)]
pub(super) enum Error {
    #[cfg(not(target_os = "linux"))]
    StartHelper {
        executable: PathBuf,
        source: io::Error,
    },
    #[cfg(not(target_os = "linux"))]
    Helper {
        executable: PathBuf,
        status: ExitStatus,
        detail: String,
    },
    #[cfg(not(target_os = "linux"))]
    NonUtf8Output(std::string::FromUtf8Error),
    #[cfg(not(target_os = "linux"))]
    UnexpectedOutput(String),
    #[cfg(target_os = "linux")]
    ReadProcess { path: PathBuf, source: io::Error },
    #[cfg(target_os = "linux")]
    MalformedProcess(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(not(target_os = "linux"))]
            Self::StartHelper { executable, source } => write!(
                formatter,
                "could not start {} to {OPERATION}: {source}",
                executable.display()
            ),
            #[cfg(not(target_os = "linux"))]
            Self::Helper {
                executable,
                status,
                detail,
            } => {
                write!(
                    formatter,
                    "could not {OPERATION}: {} exited with {status}",
                    executable.display()
                )?;
                if !detail.is_empty() {
                    write!(formatter, ": {detail}")?;
                }
                Ok(())
            }
            #[cfg(not(target_os = "linux"))]
            Self::NonUtf8Output(source) => {
                write!(formatter, "could not {OPERATION}: {source}")
            }
            #[cfg(not(target_os = "linux"))]
            Self::UnexpectedOutput(output) => write!(
                formatter,
                "could not {OPERATION}: the process helper returned unexpected output `{output}`"
            ),
            #[cfg(target_os = "linux")]
            Self::ReadProcess { path, source } => write!(
                formatter,
                "could not {OPERATION} from {}: {source}",
                path.display()
            ),
            #[cfg(target_os = "linux")]
            Self::MalformedProcess(path) => write!(
                formatter,
                "could not {OPERATION}: {} contains unexpected process metadata",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            #[cfg(not(target_os = "linux"))]
            Self::StartHelper { source, .. } => Some(source),
            #[cfg(target_os = "linux")]
            Self::ReadProcess { source, .. } => Some(source),
            #[cfg(not(target_os = "linux"))]
            Self::NonUtf8Output(source) => Some(source),
            #[cfg(not(target_os = "linux"))]
            Self::Helper { .. } | Self::UnexpectedOutput(_) => None,
            #[cfg(target_os = "linux")]
            Self::MalformedProcess(_) => None,
        }
    }
}
