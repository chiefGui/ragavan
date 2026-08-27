use crate::{
    Adapter, AdapterError, InstallEdit, Selection, UninstallEdit, profile, protocol::RUN_COMMAND,
};
use std::{
    ffi::OsStr,
    fmt, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output},
};

const PROFILE_PATH_OPERATION: &str = "locate the PowerShell profile";
const PROFILE_INTEGRATION: &[&str] = &[
    "# Managed by Ragavan. Run `ragavan uninstall powershell` to remove.",
    "$__RagavanBootstrapCommand = Get-Command ragavan -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1",
    "if ($null -ne $__RagavanBootstrapCommand) {",
    "    Invoke-Expression (& $__RagavanBootstrapCommand hook powershell | Out-String)",
    "}",
    "Remove-Variable __RagavanBootstrapCommand -ErrorAction SilentlyContinue",
];

pub(super) const ADAPTER: Adapter = Adapter {
    name: "powershell",
    display_name: "PowerShell",
    activation_command: "Invoke-Expression (ragavan hook powershell | Out-String)",
    matches,
    install,
    uninstall,
    hook,
};

const POWERSHELL_HOOK_HEADER: &str = r#"$global:__RagavanOriginalCommands = @{}
$global:__RagavanCommand = Get-Command ragavan -CommandType Application -ErrorAction Stop | Select-Object -First 1
"#;

const POWERSHELL_COMMAND_HOOK: &str = r#"
$global:__RagavanOriginalCommands['__RAGAVAN_COMMAND__'] = Get-Command '__RAGAVAN_COMMAND__' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if ($null -ne $global:__RagavanOriginalCommands['__RAGAVAN_COMMAND__']) {
    function global:__RAGAVAN_COMMAND__ {
        & $global:__RagavanCommand __RAGAVAN_RUN_COMMAND__ '__RAGAVAN_COMMAND__' $global:__RagavanOriginalCommands['__RAGAVAN_COMMAND__'].Path '0' @args
    }
}
"#;

fn hook(commands: &[&str]) -> String {
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

fn install(selection: &Selection) -> Result<InstallEdit, AdapterError> {
    Ok(PowerShell::resolve(selection)?.install()?)
}

fn uninstall(selection: &Selection) -> Result<UninstallEdit, AdapterError> {
    Ok(PowerShell::resolve(selection)?.uninstall()?)
}

struct PowerShell {
    profile: PathBuf,
}

impl PowerShell {
    fn resolve(selection: &Selection) -> Result<Self, Error> {
        match selection {
            Selection::Detected { executable } => Self::from_executable(executable),
            Selection::Explicit => Self::available(),
        }
    }

    fn available() -> Result<Self, Error> {
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

    fn install(self) -> Result<InstallEdit, profile::Error> {
        let profiles = vec![self.profile];
        let changed = !profile::install(&profiles, PROFILE_INTEGRATION)?.is_empty();
        Ok(InstallEdit { profiles, changed })
    }

    fn uninstall(self) -> Result<UninstallEdit, profile::Error> {
        Ok(UninstallEdit::from_profiles(profile::uninstall(&[
            self.profile
        ])?))
    }
}

fn available_executables() -> &'static [&'static str] {
    #[cfg(windows)]
    const EXECUTABLES: &[&str] = &["pwsh.exe", "powershell.exe"];
    #[cfg(not(windows))]
    const EXECUTABLES: &[&str] = &["pwsh", "powershell"];

    EXECUTABLES
}

fn matches(path: &Path) -> bool {
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
enum Error {
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
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StartPowerShell { source, .. } => Some(source),
            Self::NonUtf8PowerShellOutput { source, .. } => Some(source),
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
        let hook = hook(&["alpha", "beta"]);

        assert!(hook.contains("function global:alpha"));
        assert!(hook.contains("__run 'alpha'"));
        assert!(hook.contains("function global:beta"));
        assert!(hook.contains("__run 'beta'"));
    }

    #[test]
    fn installation_renders_the_powershell_bootstrap() {
        let directory = TestDirectory::new();
        let profile = directory.path().join("PowerShell").join("profile.ps1");
        let original = b"Set-Alias serve Invoke-Project";
        fs::create_dir_all(profile.parent().expect("profile should have a parent"))
            .expect("profile directory should be created");
        fs::write(&profile, original).expect("profile should be written");

        let edit = powershell(&profile)
            .install()
            .expect("install should succeed");
        assert!(edit.changed);
        assert_eq!(edit.profiles, std::slice::from_ref(&profile));
        let installed = fs::read_to_string(&profile).expect("profile should remain UTF-8");
        assert!(installed.starts_with("Set-Alias serve Invoke-Project"));
        assert!(installed.contains("Get-Command ragavan -CommandType Application"));
        assert!(installed.contains("hook powershell"));
        assert!(installed.contains("Remove-Variable __RagavanBootstrapCommand"));
        assert!(!installed.contains("hook bash"));

        let edit = powershell(&profile)
            .uninstall()
            .expect("uninstall should succeed");
        let UninstallEdit::Uninstalled { profiles } = edit else {
            panic!("installed integration should be removed");
        };
        assert_eq!(profiles, std::slice::from_ref(&profile));
        assert_eq!(
            fs::read(&profile).expect("profile should be readable"),
            original
        );
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
            let resolved = crate::resolve(crate::ShellTarget::Current)
                .expect("current-shell detection should succeed");
            assert_eq!(resolved.adapter.name, "powershell");
            let powershell = PowerShell::resolve(&resolved.selection)
                .expect("the current PowerShell profile should resolve");
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

    fn powershell(profile: &Path) -> PowerShell {
        PowerShell {
            profile: profile.to_owned(),
        }
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
