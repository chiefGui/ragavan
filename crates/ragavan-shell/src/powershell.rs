mod profile;

use self::profile::Profile;
use crate::{InstallOutcome, UninstallOutcome, protocol::RUN_COMMAND};
use std::{
    ffi::OsStr,
    fmt, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output},
};

const PROFILE_PATH_OPERATION: &str = "locate the PowerShell profile";
const CURRENT_SHELL_OPERATION: &str = "identify the current shell";
#[cfg(windows)]
const NOT_POWERSHELL_EXIT_CODE: i32 = 4;

const POWERSHELL_HOOK_HEADER: &str = r#"$global:__RagavanOriginalCommands = @{}
$global:__RagavanCommand = Get-Command ragavan -CommandType Application -ErrorAction Stop | Select-Object -First 1
"#;

const POWERSHELL_COMMAND_HOOK: &str = r#"
$global:__RagavanOriginalCommands['__RAGAVAN_COMMAND__'] = Get-Command '__RAGAVAN_COMMAND__' -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if ($null -ne $global:__RagavanOriginalCommands['__RAGAVAN_COMMAND__']) {
    function global:__RAGAVAN_COMMAND__ {
        & $global:__RagavanCommand __RAGAVAN_RUN_COMMAND__ '__RAGAVAN_COMMAND__' $global:__RagavanOriginalCommands['__RAGAVAN_COMMAND__'].Path @args
    }
}
"#;

pub(super) fn hook<'a>(commands: impl IntoIterator<Item = &'a str>) -> String {
    let mut hook = POWERSHELL_HOOK_HEADER.to_owned();
    for command in commands {
        assert!(
            command
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "shell command names must be safe PowerShell identifiers"
        );
        hook.push_str(
            &POWERSHELL_COMMAND_HOOK
                .replace("__RAGAVAN_COMMAND__", command)
                .replace("__RAGAVAN_RUN_COMMAND__", RUN_COMMAND),
        );
    }
    hook
}

pub(super) struct PowerShell {
    profile: PathBuf,
}

impl PowerShell {
    pub(super) fn current() -> Result<Option<Self>, Error> {
        let Some(executable) = current_executable()? else {
            return Ok(None);
        };

        Self::from_executable(&executable).map(Some)
    }

    pub(super) fn available() -> Result<Self, Error> {
        for executable in available_executables() {
            match Self::from_executable(Path::new(executable)) {
                Err(Error::StartPowerShell { source, .. })
                    if source.kind() == io::ErrorKind::NotFound => {}
                result => return result,
            }
        }

        Err(Error::PowerShellNotFound)
    }

    fn from_executable(executable: &Path) -> Result<Self, Error> {
        let output = Command::new(executable)
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .arg(
                "[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); \
                 [Console]::Out.Write($PROFILE.CurrentUserAllHosts)",
            )
            .output()
            .map_err(|source| Error::StartPowerShell {
                operation: PROFILE_PATH_OPERATION,
                executable: executable.to_owned(),
                source,
            })?;
        let profile = successful_output(PROFILE_PATH_OPERATION, executable, output)?;
        if profile.is_empty() || profile.contains(['\r', '\n', '\0']) {
            return Err(Error::UnexpectedPowerShellOutput {
                operation: PROFILE_PATH_OPERATION,
                output: profile,
            });
        }

        let profile = PathBuf::from(profile);
        if !profile.is_absolute() {
            return Err(Error::UnexpectedPowerShellOutput {
                operation: PROFILE_PATH_OPERATION,
                output: profile.display().to_string(),
            });
        }

        Ok(Self { profile })
    }

    pub(super) fn install(self) -> Result<InstallOutcome, Error> {
        let mut profile =
            Profile::read(&self.profile)?.unwrap_or_else(|| Profile::empty(&self.profile));
        let changed = profile.install()?;

        if changed {
            Ok(InstallOutcome::Installed {
                profile: self.profile,
            })
        } else {
            Ok(InstallOutcome::AlreadyInstalled {
                profile: self.profile,
            })
        }
    }

    pub(super) fn uninstall(self) -> Result<UninstallOutcome, Error> {
        let Some(mut profile) = Profile::read(&self.profile)? else {
            return Ok(UninstallOutcome::AlreadyUninstalled {
                profile: self.profile,
            });
        };
        let changed = profile.uninstall()?;

        if changed {
            Ok(UninstallOutcome::Uninstalled {
                profile: self.profile,
            })
        } else {
            Ok(UninstallOutcome::AlreadyUninstalled {
                profile: self.profile,
            })
        }
    }
}

fn available_executables() -> &'static [&'static str] {
    #[cfg(windows)]
    const EXECUTABLES: &[&str] = &["pwsh.exe", "powershell.exe"];
    #[cfg(not(windows))]
    const EXECUTABLES: &[&str] = &["pwsh", "powershell"];

    EXECUTABLES
}

#[cfg(windows)]
fn current_executable() -> Result<Option<PathBuf>, Error> {
    let process_id = std::process::id();
    let script = format!(
        "$process = Get-CimInstance Win32_Process -Filter 'ProcessId = {process_id}'; \
         $parent = Get-CimInstance Win32_Process -Filter \"ProcessId = $($process.ParentProcessId)\"; \
         if ($parent.Name -notin @('powershell.exe', 'pwsh.exe')) {{ exit {NOT_POWERSHELL_EXIT_CODE} }}; \
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
        .map_err(|source| Error::StartPowerShell {
            operation: CURRENT_SHELL_OPERATION,
            executable: helper.clone(),
            source,
        })?;

    if output.status.code() == Some(NOT_POWERSHELL_EXIT_CODE) {
        return Ok(None);
    }

    let executable = successful_output(CURRENT_SHELL_OPERATION, &helper, output)?;
    if executable.is_empty() {
        return Err(Error::UnexpectedPowerShellOutput {
            operation: CURRENT_SHELL_OPERATION,
            output: executable,
        });
    }

    let executable = PathBuf::from(executable);
    if is_powershell(&executable) {
        Ok(Some(executable))
    } else {
        Err(Error::UnexpectedPowerShellOutput {
            operation: CURRENT_SHELL_OPERATION,
            output: executable.display().to_string(),
        })
    }
}

#[cfg(not(windows))]
fn current_executable() -> Result<Option<PathBuf>, Error> {
    let process_id = std::process::id().to_string();
    let parent = detection_output(
        Command::new("ps")
            .args(["-p", &process_id, "-o", "ppid="])
            .output(),
    )?;
    let parent = parent.trim();
    if parent.is_empty() || !parent.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::UnexpectedPowerShellOutput {
            operation: CURRENT_SHELL_OPERATION,
            output: parent.to_owned(),
        });
    }

    let command = detection_output(
        Command::new("ps")
            .args(["-p", parent, "-o", "comm="])
            .output(),
    )?;
    let command = Path::new(command.trim());

    if is_powershell(command) {
        Ok(Some(command.to_owned()))
    } else {
        Ok(None)
    }
}

#[cfg(not(windows))]
fn detection_output(output: io::Result<Output>) -> Result<String, Error> {
    let output = output.map_err(|source| Error::StartPowerShell {
        operation: CURRENT_SHELL_OPERATION,
        executable: PathBuf::from("ps"),
        source,
    })?;
    successful_output(CURRENT_SHELL_OPERATION, Path::new("ps"), output)
}

fn is_powershell(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("powershell")
                || name.eq_ignore_ascii_case("powershell.exe")
                || name.eq_ignore_ascii_case("pwsh")
                || name.eq_ignore_ascii_case("pwsh.exe")
        })
}

fn successful_output(
    operation: &'static str,
    executable: &Path,
    output: Output,
) -> Result<String, Error> {
    if !output.status.success() {
        return Err(Error::PowerShell {
            operation,
            executable: executable.to_owned(),
            status: output.status,
            detail: output_detail(&output),
        });
    }

    String::from_utf8(output.stdout)
        .map_err(|source| Error::NonUtf8PowerShellOutput { operation, source })
}

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
    StartPowerShell {
        operation: &'static str,
        executable: PathBuf,
        source: io::Error,
    },
    PowerShell {
        operation: &'static str,
        executable: PathBuf,
        status: ExitStatus,
        detail: String,
    },
    NonUtf8PowerShellOutput {
        operation: &'static str,
        source: std::string::FromUtf8Error,
    },
    UnexpectedPowerShellOutput {
        operation: &'static str,
        output: String,
    },
    PowerShellNotFound,
    InvalidProfilePath {
        path: PathBuf,
    },
    ReadProfile {
        path: PathBuf,
        source: io::Error,
    },
    DecodeProfile {
        path: PathBuf,
        source: profile::DecodeError,
    },
    MalformedProfile {
        path: PathBuf,
    },
    ProfileChanged {
        path: PathBuf,
    },
    WriteProfile {
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartPowerShell {
                operation,
                executable,
                source,
            } => write!(
                formatter,
                "could not start {} to {operation}: {source}",
                executable.display()
            ),
            Self::PowerShell {
                operation,
                executable,
                status,
                detail,
            } => {
                write!(
                    formatter,
                    "could not {operation}: {} exited with {status}",
                    executable.display()
                )?;
                if !detail.is_empty() {
                    write!(formatter, ": {detail}")?;
                }
                Ok(())
            }
            Self::NonUtf8PowerShellOutput { operation, source } => {
                write!(formatter, "could not {operation}: {source}")
            }
            Self::UnexpectedPowerShellOutput { operation, output } => write!(
                formatter,
                "could not {operation}: PowerShell returned unexpected output `{output}`"
            ),
            Self::PowerShellNotFound => formatter.write_str(
                "could not find PowerShell; install `pwsh` or make `powershell` available on PATH",
            ),
            Self::InvalidProfilePath { path } => write!(
                formatter,
                "could not update PowerShell profile: {} is not a file path",
                path.display()
            ),
            Self::ReadProfile { path, source } => {
                write!(formatter, "could not read {}: {source}", path.display())
            }
            Self::DecodeProfile { path, source } => write!(
                formatter,
                "could not safely edit {} because its text encoding is unsupported: {source}",
                path.display()
            ),
            Self::MalformedProfile { path } => write!(
                formatter,
                "could not safely edit {} because its Ragavan markers are incomplete, duplicated, or out of order",
                path.display()
            ),
            Self::ProfileChanged { path } => write!(
                formatter,
                "could not update {} because it changed while Ragavan was editing it; rerun the command",
                path.display()
            ),
            Self::WriteProfile { path, source } => {
                write!(formatter, "could not update {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StartPowerShell { source, .. } => Some(source),
            Self::NonUtf8PowerShellOutput { source, .. } => Some(source),
            Self::ReadProfile { source, .. } => Some(source),
            Self::DecodeProfile { source, .. } => Some(source),
            Self::WriteProfile { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn hook_renders_every_registered_command_through_one_protocol() {
        let hook = hook(["alpha", "beta"]);

        assert!(hook.contains("function global:alpha"));
        assert!(hook.contains("__run 'alpha'"));
        assert!(hook.contains("function global:beta"));
        assert!(hook.contains("__run 'beta'"));
    }

    #[test]
    fn installation_is_idempotent_and_uninstallation_restores_user_content() {
        let directory = TestDirectory::new();
        let profile = directory.path().join("PowerShell").join("profile.ps1");
        let original = b"Set-Alias serve Invoke-Project";
        fs::create_dir_all(profile.parent().expect("profile should have a parent"))
            .expect("profile directory should be created");
        fs::write(&profile, original).expect("profile should be written");

        assert_eq!(
            powershell(&profile)
                .install()
                .expect("install should succeed"),
            InstallOutcome::Installed {
                profile: profile.clone()
            }
        );
        let installed = fs::read_to_string(&profile).expect("profile should remain UTF-8");
        assert!(installed.starts_with("Set-Alias serve Invoke-Project"));
        assert_eq!(installed.matches("# >>> ragavan >>>").count(), 1);
        assert_eq!(installed.matches("# <<< ragavan <<<").count(), 1);
        assert!(installed.contains("hook powershell"));

        let installed_bytes = fs::read(&profile).expect("profile should be readable");
        assert_eq!(
            powershell(&profile)
                .install()
                .expect("reinstall should succeed"),
            InstallOutcome::AlreadyInstalled {
                profile: profile.clone()
            }
        );
        assert_eq!(
            fs::read(&profile).expect("profile should be readable"),
            installed_bytes
        );

        assert_eq!(
            powershell(&profile)
                .uninstall()
                .expect("uninstall should succeed"),
            UninstallOutcome::Uninstalled {
                profile: profile.clone()
            }
        );
        assert_eq!(
            fs::read(&profile).expect("profile should be readable"),
            original
        );
        assert_eq!(
            powershell(&profile)
                .uninstall()
                .expect("repeated uninstall should succeed"),
            UninstallOutcome::AlreadyUninstalled { profile }
        );
    }

    #[test]
    fn installation_updates_only_the_owned_block() {
        let directory = TestDirectory::new();
        let profile = directory.path().join("profile.ps1");
        let original = concat!(
            "$before = 'user-owned'\r\n",
            "\r\n",
            "# >>> ragavan >>>\r\n",
            "stale integration\r\n",
            "# <<< ragavan <<<\r\n",
            "$after = 'also-user-owned'\r\n",
        );
        fs::write(&profile, original).expect("profile should be written");

        assert!(matches!(
            powershell(&profile)
                .install()
                .expect("install should succeed"),
            InstallOutcome::Installed { .. }
        ));
        let installed = fs::read_to_string(&profile).expect("profile should be readable");
        assert!(installed.starts_with("$before = 'user-owned'\r\n\r\n"));
        assert!(installed.ends_with("$after = 'also-user-owned'\r\n"));
        assert!(!installed.contains("stale integration"));
        assert!(!installed.replace("\r\n", "").contains('\n'));

        powershell(&profile)
            .uninstall()
            .expect("uninstall should succeed");
        assert_eq!(
            fs::read_to_string(&profile).expect("profile should be readable"),
            "$before = 'user-owned'\r\n$after = 'also-user-owned'\r\n"
        );
    }

    #[test]
    fn malformed_markers_are_rejected_without_modifying_the_profile() {
        let directory = TestDirectory::new();
        let profile = directory.path().join("profile.ps1");
        let original = b"$before = 1\n# >>> ragavan >>>\nmissing end marker\n";
        fs::write(&profile, original).expect("profile should be written");

        let error = powershell(&profile)
            .install()
            .expect_err("malformed integration should fail");
        assert!(error.to_string().contains("markers are incomplete"));
        assert_eq!(
            fs::read(&profile).expect("profile should be readable"),
            original
        );
        assert_eq!(
            fs::read_dir(directory.path())
                .expect("test directory should be readable")
                .count(),
            1
        );
    }

    #[test]
    fn utf16_encoding_is_preserved() {
        let directory = TestDirectory::new();
        let profile = directory.path().join("profile.ps1");
        let original = utf16_little_endian("$message = 'olá'\r\n");
        fs::write(&profile, &original).expect("profile should be written");

        powershell(&profile)
            .install()
            .expect("install should succeed");
        let installed = fs::read(&profile).expect("profile should be readable");
        assert!(installed.starts_with(&[0xff, 0xfe]));
        assert!(decode_utf16_little_endian(&installed).contains("hook powershell"));

        powershell(&profile)
            .uninstall()
            .expect("uninstall should succeed");
        assert_eq!(
            fs::read(&profile).expect("profile should be readable"),
            original
        );
    }

    #[test]
    fn unsupported_encoding_is_rejected_without_modifying_the_profile() {
        let directory = TestDirectory::new();
        let profile = directory.path().join("profile.ps1");
        let original = [0x80, 0x81];
        fs::write(&profile, original).expect("profile should be written");

        let error = powershell(&profile)
            .install()
            .expect_err("unsupported encoding should fail");
        assert!(error.to_string().contains("text encoding is unsupported"));
        assert_eq!(
            fs::read(&profile).expect("profile should be readable"),
            original
        );
    }

    #[test]
    fn installation_creates_a_missing_profile_and_parent_directory() {
        let directory = TestDirectory::new();
        let profile = directory
            .path()
            .join("missing")
            .join("PowerShell")
            .join("profile.ps1");

        assert!(matches!(
            powershell(&profile)
                .install()
                .expect("install should succeed"),
            InstallOutcome::Installed { .. }
        ));
        let installed = fs::read_to_string(&profile).expect("profile should be readable");
        assert!(installed.starts_with("# >>> ragavan >>>"));
        assert!(installed.ends_with(if cfg!(windows) { "\r\n" } else { "\n" }));
    }

    #[cfg(windows)]
    #[test]
    fn powershell_reports_an_absolute_all_hosts_profile() {
        let powershell = PowerShell::available().expect("Windows should provide PowerShell");

        assert!(powershell.profile.is_absolute());
        assert_eq!(
            powershell.profile.file_name().and_then(OsStr::to_str),
            Some("profile.ps1")
        );
    }

    #[cfg(windows)]
    #[test]
    fn current_powershell_is_detected() {
        const CHILD: &str = "RAGAVAN_CURRENT_POWERSHELL_TEST";
        const TEST: &str = "powershell::tests::current_powershell_is_detected";

        if std::env::var_os(CHILD).is_some() {
            let powershell = PowerShell::current()
                .expect("current-shell detection should succeed")
                .expect("the direct parent should be PowerShell");
            assert!(powershell.profile.is_absolute());
            return;
        }

        let test_executable = std::env::current_exe().expect("test executable should be known");
        let output = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "& $env:RAGAVAN_TEST_EXECUTABLE --exact powershell::tests::current_powershell_is_detected --nocapture",
            ])
            .env(CHILD, "1")
            .env("RAGAVAN_TEST_EXECUTABLE", test_executable)
            .output()
            .expect("PowerShell should start");

        assert!(
            output.status.success(),
            "nested detection test should pass: {output:?}"
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(TEST));
    }

    #[cfg(unix)]
    #[test]
    fn installation_preserves_a_linked_profile() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let target = directory.path().join("dotfiles-profile.ps1");
        let profile = directory.path().join("profile.ps1");
        let original = b"$ownedBy = 'dotfiles'\n";
        fs::write(&target, original).expect("profile target should be written");
        symlink(&target, &profile).expect("profile link should be created");

        powershell(&profile)
            .install()
            .expect("install should succeed");
        assert!(
            fs::symlink_metadata(&profile)
                .expect("profile link should exist")
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::read_to_string(&target)
                .expect("profile target should be readable")
                .contains("hook powershell")
        );

        powershell(&profile)
            .uninstall()
            .expect("uninstall should succeed");
        assert_eq!(
            fs::read(&target).expect("profile target should be readable"),
            original
        );
    }

    fn powershell(profile: &Path) -> PowerShell {
        PowerShell {
            profile: profile.to_owned(),
        }
    }

    fn utf16_little_endian(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xfe];
        for code_unit in text.encode_utf16() {
            bytes.extend_from_slice(&code_unit.to_le_bytes());
        }
        bytes
    }

    fn decode_utf16_little_endian(bytes: &[u8]) -> String {
        assert!(bytes.starts_with(&[0xff, 0xfe]));
        let code_units: Vec<_> = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16(&code_units).expect("profile should remain valid UTF-16")
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..100 {
                let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "ragavan-shell-test-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("could not create test directory {path:?}: {error}"),
                }
            }

            panic!("could not allocate a unique test directory");
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                eprintln!("could not remove test directory {:?}: {error}", self.0);
            }
        }
    }
}
