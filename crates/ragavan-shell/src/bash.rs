use crate::{
    Adapter, AdapterError, InstallEdit, Selection, UninstallEdit, adapter_error, profile,
    protocol::RUN_COMMAND,
};
use ragavan_diagnostics::{Detail, Diagnostic};
use std::{
    fmt,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
};

const LOGIN_PROFILE_NAMES: [&str; 3] = [".bash_profile", ".bash_login", ".profile"];

const PROFILE_INTEGRATION: &[&str] = &[
    "# Managed by Ragavan. Run `ragavan uninstall bash` to remove.",
    "if [ -n \"${BASH_VERSION:-}\" ]; then",
    "    if builtin type -P ragavan >/dev/null 2>&1; then",
    "        eval \"$(command ragavan hook bash)\"",
    "    fi",
    "fi",
];

pub(super) const ADAPTER: Adapter = Adapter {
    name: "bash",
    display_name: "Bash",
    activation_command: "eval \"$(ragavan hook bash)\"",
    matches,
    install,
    uninstall,
    hook,
};

fn matches(executable: &Path) -> bool {
    executable
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.strip_prefix('-').unwrap_or(name))
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("bash") || name.eq_ignore_ascii_case("bash.exe")
        })
}

fn install(_: &Selection) -> Result<InstallEdit, AdapterError> {
    Bash::current()
        .and_then(Bash::install)
        .map_err(adapter_error)
}

fn uninstall(_: &Selection) -> Result<UninstallEdit, AdapterError> {
    Bash::current()
        .and_then(Bash::uninstall)
        .map_err(adapter_error)
}

fn hook(native_executable: &str, commands: &[&str]) -> String {
    let mut hook = format!(
        "__RagavanExecutable={}\ndeclare -a __RagavanOriginalCommands=()\n",
        shell_literal(native_executable)
    );
    for (index, command) in commands.iter().enumerate() {
        assert!(
            command
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "shell command names must be safe Bash function names"
        );
        writeln!(
            hook,
            "if __RagavanOriginalCommands[{index}]=\"$(builtin type -P {command})\"; then"
        )
        .expect("writing to a string cannot fail");
        writeln!(hook, "    function {command} {{").expect("writing to a string cannot fail");
        #[cfg(windows)]
        writeln!(
            hook,
            "        \"${{__RagavanExecutable}}\" {RUN_COMMAND} '{command}' \"${{BASH}}\" '3' '-c' 'exec \"$0\" \"$@\"' \"${{__RagavanOriginalCommands[{index}]}}\" \"$@\""
        )
        .expect("writing to a string cannot fail");
        #[cfg(not(windows))]
        writeln!(
            hook,
            "        \"${{__RagavanExecutable}}\" {RUN_COMMAND} '{command}' \"${{__RagavanOriginalCommands[{index}]}}\" '0' \"$@\""
        )
        .expect("writing to a string cannot fail");
        hook.push_str("    }\nfi\n");
    }
    hook
}

fn shell_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

struct Bash {
    home: PathBuf,
}

impl Bash {
    fn current() -> Result<Self, Error> {
        let home = std::env::var_os("HOME").filter(|home| !home.is_empty());
        #[cfg(windows)]
        let home =
            home.or_else(|| std::env::var_os("USERPROFILE").filter(|profile| !profile.is_empty()));
        let home = home.ok_or(Error::MissingHome)?;
        Self::from_home(Path::new(&home))
    }

    fn from_home(home: &Path) -> Result<Self, Error> {
        if !home.is_absolute() {
            return Err(Error::InvalidHome(home.to_owned()));
        }
        Ok(Self {
            home: home.to_owned(),
        })
    }

    fn install(self) -> Result<InstallEdit, Error> {
        let profiles = self.installation_profiles()?;
        let changed = !profile::install(&profiles, PROFILE_INTEGRATION)?.is_empty();
        Ok(InstallEdit { profiles, changed })
    }

    fn uninstall(self) -> Result<UninstallEdit, Error> {
        Ok(UninstallEdit::from_profiles(profile::uninstall(
            &self.profile_candidates(),
        )?))
    }

    fn installation_profiles(&self) -> Result<Vec<PathBuf>, Error> {
        Ok(vec![self.home.join(".bashrc"), self.login_profile()?])
    }

    fn login_profile(&self) -> Result<PathBuf, Error> {
        for name in LOGIN_PROFILE_NAMES {
            let profile = self.home.join(name);
            match fs::File::open(&profile) {
                Ok(file) => {
                    let metadata = file.metadata().map_err(|source| Error::InspectProfile {
                        profile: profile.clone(),
                        source,
                    })?;
                    if metadata.is_file() {
                        return Ok(profile);
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound
                            | io::ErrorKind::PermissionDenied
                            | io::ErrorKind::IsADirectory
                    ) => {}
                Err(source) => return Err(Error::InspectProfile { profile, source }),
            }
        }

        Ok(self
            .home
            .join(LOGIN_PROFILE_NAMES[LOGIN_PROFILE_NAMES.len() - 1]))
    }

    fn profile_candidates(&self) -> Vec<PathBuf> {
        std::iter::once(self.home.join(".bashrc"))
            .chain(LOGIN_PROFILE_NAMES.map(|name| self.home.join(name)))
            .collect()
    }
}

#[derive(Debug)]
enum Error {
    MissingHome,
    InvalidHome(PathBuf),
    InspectProfile { profile: PathBuf, source: io::Error },
    UpdateProfile(profile::Error),
}

impl From<profile::Error> for Error {
    fn from(error: profile::Error) -> Self {
        Self::UpdateProfile(error)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHome => formatter.write_str(if cfg!(windows) {
                "could not locate the Bash profile because HOME and USERPROFILE are not set"
            } else {
                "could not locate the Bash profile because HOME is not set"
            }),
            Self::InvalidHome(home) => write!(
                formatter,
                "could not locate the Bash profile because the home directory is not an absolute path: {}",
                home.display()
            ),
            Self::InspectProfile { profile, source } => {
                write!(
                    formatter,
                    "could not inspect {}: {source}",
                    profile.display()
                )
            }
            Self::UpdateProfile(source) => source.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InspectProfile { source, .. } => Some(source),
            Self::UpdateProfile(source) => Some(source),
            Self::MissingHome | Self::InvalidHome(_) => None,
        }
    }
}

impl Diagnostic for Error {
    fn code(&self) -> &'static str {
        match self {
            Self::MissingHome => "shell.bash.home.missing",
            Self::InvalidHome(_) => "shell.bash.home.invalid",
            Self::InspectProfile { .. } => "shell.bash.profile.inspect",
            Self::UpdateProfile(source) => source.code(),
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            Self::MissingHome => Some(if cfg!(windows) {
                "set HOME or USERPROFILE to an absolute home directory".to_owned()
            } else {
                "set HOME to an absolute home directory".to_owned()
            }),
            Self::InvalidHome(_) => {
                Some("set the home-directory environment variable to an absolute path".to_owned())
            }
            Self::InspectProfile { .. } => None,
            Self::UpdateProfile(source) => source.help(),
        }
    }

    fn details(&self) -> Vec<Detail> {
        match self {
            Self::MissingHome => Vec::new(),
            Self::InvalidHome(home) => {
                vec![Detail::text("home", home.display().to_string())]
            }
            Self::InspectProfile { profile, .. } => {
                vec![Detail::text("profile", profile.display().to_string())]
            }
            Self::UpdateProfile(source) => source.details(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs, io,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn hook_routes_every_registered_command_through_one_protocol() {
        let hook = hook("/native/ragavan", &["alpha", "beta"]);

        assert!(hook.contains("function alpha"));
        #[cfg(windows)]
        assert!(hook.contains(
            "\"${__RagavanExecutable}\" __run 'alpha' \"${BASH}\" '3' '-c' 'exec \"$0\" \"$@\"' \"${__RagavanOriginalCommands[0]}\" \"$@\""
        ));
        #[cfg(not(windows))]
        assert!(hook.contains(
            "\"${__RagavanExecutable}\" __run 'alpha' \"${__RagavanOriginalCommands[0]}\" '0' \"$@\""
        ));
        assert!(hook.contains("function beta"));
        #[cfg(windows)]
        assert!(hook.contains(
            "\"${__RagavanExecutable}\" __run 'beta' \"${BASH}\" '3' '-c' 'exec \"$0\" \"$@\"' \"${__RagavanOriginalCommands[1]}\" \"$@\""
        ));
        #[cfg(not(windows))]
        assert!(hook.contains(
            "\"${__RagavanExecutable}\" __run 'beta' \"${__RagavanOriginalCommands[1]}\" '0' \"$@\""
        ));
    }

    #[test]
    fn executable_matching_accepts_regular_and_login_bash_names() {
        assert!(matches(Path::new("bash")));
        assert!(matches(Path::new("/usr/bin/bash")));
        assert!(matches(Path::new("-bash")));
        assert!(matches(Path::new("C:/Program Files/Git/bin/bash.exe")));
        assert!(!matches(Path::new("zsh")));
    }

    #[test]
    fn missing_login_profile_selects_the_lowest_precedence_profile() {
        let directory = TestDirectory::new();
        let bash = Bash::from_home(directory.path()).expect("absolute HOME should be accepted");

        assert_eq!(
            bash.installation_profiles()
                .expect("profiles should be selected"),
            [
                directory.path().join(".bashrc"),
                directory.path().join(".profile"),
            ]
        );
    }

    #[test]
    fn installation_covers_interactive_and_login_bash_without_shadowing_profile() {
        let directory = TestDirectory::new();
        let bashrc = directory.path().join(".bashrc");
        let login = directory.path().join(".profile");
        let bashrc_original = b"alias serve=project\n";
        let login_original = b"export PROJECT_MODE=development\n";
        fs::write(&bashrc, bashrc_original).expect("Bash profile should be written");
        fs::write(&login, login_original).expect("login profile should be written");

        let edit = bash(directory.path())
            .install()
            .expect("install should succeed");
        assert!(edit.changed);
        assert_eq!(edit.profiles, [bashrc.clone(), login.clone()]);
        assert!(!directory.path().join(".bash_profile").exists());

        let installed_bashrc = fs::read_to_string(&bashrc).expect("Bash profile should be UTF-8");
        let installed_login = fs::read_to_string(&login).expect("login profile should be UTF-8");
        assert!(installed_bashrc.starts_with("alias serve=project\n"));
        assert!(installed_login.starts_with("export PROJECT_MODE=development\n"));
        for installed in [&installed_bashrc, &installed_login] {
            assert!(installed.contains("[ -n \"${BASH_VERSION:-}\" ]"));
            assert!(installed.contains("builtin type -P ragavan"));
            assert!(installed.contains("ragavan hook bash"));
            assert!(!installed.contains("hook powershell"));
        }

        let edit = bash(directory.path())
            .uninstall()
            .expect("uninstall should succeed");
        let UninstallEdit::Uninstalled { profiles } = edit else {
            panic!("installed integration should be removed");
        };
        assert_eq!(profiles, [bashrc.clone(), login.clone()]);
        assert_eq!(
            fs::read(&bashrc).expect("profile should be readable"),
            bashrc_original
        );
        assert_eq!(
            fs::read(&login).expect("profile should be readable"),
            login_original
        );
    }

    #[cfg(unix)]
    #[test]
    fn integration_in_profile_is_safe_for_non_bash_shells() {
        let directory = TestDirectory::new();
        fs::write(
            directory.path().join(".profile"),
            "TEST_USER_CONTENT=preserved\n",
        )
        .expect("profile should be written");
        bash(directory.path())
            .install()
            .expect("install should succeed");

        let output = std::process::Command::new("sh")
            .args(["-c", ". \"$HOME/.profile\""])
            .env("HOME", directory.path())
            .env_remove("BASH_VERSION")
            .output()
            .expect("a POSIX shell should start");

        assert!(
            output.status.success(),
            "a non-Bash shell should read .profile without integration errors: {output:?}"
        );
    }

    #[test]
    fn login_profile_follows_bash_precedence() {
        let cases: &[(&[&str], &str)] = &[
            (&[".profile"], ".profile"),
            (&[".profile", ".bash_login"], ".bash_login"),
            (
                &[".profile", ".bash_login", ".bash_profile"],
                ".bash_profile",
            ),
        ];

        for &(present, expected) in cases {
            let directory = TestDirectory::new();
            for name in present {
                fs::write(
                    directory.path().join(name),
                    format!("user content for {name}\n"),
                )
                .expect("login profile should be written");
            }

            let profiles = bash(directory.path())
                .installation_profiles()
                .expect("profiles should be selected");
            assert_eq!(profiles[1], directory.path().join(expected));
        }
    }

    #[cfg(unix)]
    #[test]
    fn login_profile_skips_a_dangling_higher_precedence_link() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        symlink(
            directory.path().join("missing-target"),
            directory.path().join(".bash_profile"),
        )
        .expect("dangling profile link should be created");
        fs::write(directory.path().join(".profile"), "readable profile\n")
            .expect("readable profile should be written");

        let profiles = bash(directory.path())
            .installation_profiles()
            .expect("profiles should be selected");

        assert_eq!(profiles[1], directory.path().join(".profile"));
    }

    #[test]
    fn uninstall_removes_integration_left_by_login_profile_precedence_changes() {
        const PROFILE_ORIGINAL: &str = "original profile\n";
        const BASH_PROFILE_ORIGINAL: &str = "new higher-priority profile\n";

        let directory = TestDirectory::new();
        let profile = directory.path().join(".profile");
        fs::write(&profile, PROFILE_ORIGINAL).expect("profile should be written");
        bash(directory.path())
            .install()
            .expect("initial install should succeed");

        let bash_profile = directory.path().join(".bash_profile");
        fs::write(&bash_profile, BASH_PROFILE_ORIGINAL)
            .expect("higher-priority profile should be written");
        bash(directory.path())
            .install()
            .expect("reinstall should follow current precedence");

        let edit = bash(directory.path())
            .uninstall()
            .expect("uninstall should succeed");
        let UninstallEdit::Uninstalled { profiles: removed } = edit else {
            panic!("installed integration should be removed");
        };
        assert_eq!(
            removed,
            [
                directory.path().join(".bashrc"),
                bash_profile.clone(),
                profile.clone(),
            ]
        );
        assert_eq!(
            fs::read_to_string(directory.path().join(".bashrc"))
                .expect("Bash profile should be readable"),
            ""
        );
        assert_eq!(
            fs::read_to_string(bash_profile).expect("Bash profile should be readable"),
            BASH_PROFILE_ORIGINAL
        );
        assert_eq!(
            fs::read_to_string(profile).expect("profile should be readable"),
            PROFILE_ORIGINAL
        );
    }

    #[cfg(unix)]
    #[test]
    fn current_bash_is_detected() {
        const CHILD: &str = "RAGAVAN_CURRENT_BASH_TEST";
        const TEST: &str = "bash::tests::current_bash_is_detected";

        if std::env::var_os(CHILD).is_some() {
            let resolved = crate::resolve(crate::ShellTarget::Current)
                .expect("current-shell detection should succeed");
            assert_eq!(resolved.adapter.name, "bash");
            let Selection::Detected { executable } = resolved.selection else {
                panic!("the current shell should retain its detected executable");
            };
            assert!(matches(&executable));
            return;
        }

        let test_executable = std::env::current_exe().expect("test executable should be known");
        let output = std::process::Command::new("bash")
            .args([
                "--noprofile",
                "--norc",
                "-c",
                "\"$RAGAVAN_TEST_EXECUTABLE\" --exact bash::tests::current_bash_is_detected --nocapture; status=$?; :; exit $status",
            ])
            .env(CHILD, "1")
            .env("RAGAVAN_TEST_EXECUTABLE", test_executable)
            .output()
            .expect("Bash should start");

        assert!(
            output.status.success(),
            "nested detection test should pass: {output:?}"
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(TEST));
    }

    fn bash(home: &Path) -> Bash {
        Bash {
            home: home.to_owned(),
        }
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..100 {
                let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "ragavan-bash-test-{}-{sequence}",
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
