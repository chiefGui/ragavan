use crate::{Adapter, AdapterError, ProfileEdit, Selection, profile, protocol::RUN_COMMAND};
use std::{
    fmt,
    fmt::Write as _,
    path::{Path, PathBuf},
};

const PROFILE_INTEGRATION: &[&str] = &[
    "# Managed by Ragavan. Run `ragavan uninstall bash` to remove.",
    "if builtin type -P ragavan >/dev/null 2>&1; then",
    "    eval \"$(command ragavan hook bash)\"",
    "fi",
];

const HOOK_HEADER: &str = concat!(
    "__RagavanCommand=\"$(builtin type -P ragavan)\"\n",
    "declare -a __RagavanOriginalCommands=()\n",
);

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

fn install(_: &Selection) -> Result<ProfileEdit, AdapterError> {
    Ok(Bash::current()?.install()?)
}

fn uninstall(_: &Selection) -> Result<ProfileEdit, AdapterError> {
    Ok(Bash::current()?.uninstall()?)
}

fn hook(commands: &[&str]) -> String {
    let mut hook = HOOK_HEADER.to_owned();
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
            "        \"${{__RagavanCommand}}\" {RUN_COMMAND} '{command}' \"${{BASH}}\" '3' '-c' 'exec \"$0\" \"$@\"' \"${{__RagavanOriginalCommands[{index}]}}\" \"$@\""
        )
        .expect("writing to a string cannot fail");
        #[cfg(not(windows))]
        writeln!(
            hook,
            "        \"${{__RagavanCommand}}\" {RUN_COMMAND} '{command}' \"${{__RagavanOriginalCommands[{index}]}}\" '0' \"$@\""
        )
        .expect("writing to a string cannot fail");
        hook.push_str("    }\nfi\n");
    }
    hook
}

struct Bash {
    profile: PathBuf,
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
            profile: home.join(".bashrc"),
        })
    }

    fn install(self) -> Result<ProfileEdit, profile::Error> {
        let changed = profile::install(&self.profile, PROFILE_INTEGRATION)?;
        Ok(ProfileEdit {
            profile: self.profile,
            changed,
        })
    }

    fn uninstall(self) -> Result<ProfileEdit, profile::Error> {
        let changed = profile::uninstall(&self.profile)?;
        Ok(ProfileEdit {
            profile: self.profile,
            changed,
        })
    }
}

#[derive(Debug)]
enum Error {
    MissingHome,
    InvalidHome(PathBuf),
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
        }
    }
}

impl std::error::Error for Error {}

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
        let hook = hook(&["alpha", "beta"]);

        assert!(hook.starts_with("__RagavanCommand=\"$(builtin type -P ragavan)\"\n"));
        assert!(hook.contains("function alpha"));
        #[cfg(windows)]
        assert!(hook.contains(
            "\"${__RagavanCommand}\" __run 'alpha' \"${BASH}\" '3' '-c' 'exec \"$0\" \"$@\"' \"${__RagavanOriginalCommands[0]}\" \"$@\""
        ));
        #[cfg(not(windows))]
        assert!(hook.contains(
            "\"${__RagavanCommand}\" __run 'alpha' \"${__RagavanOriginalCommands[0]}\" '0' \"$@\""
        ));
        assert!(hook.contains("function beta"));
        #[cfg(windows)]
        assert!(hook.contains(
            "\"${__RagavanCommand}\" __run 'beta' \"${BASH}\" '3' '-c' 'exec \"$0\" \"$@\"' \"${__RagavanOriginalCommands[1]}\" \"$@\""
        ));
        #[cfg(not(windows))]
        assert!(hook.contains(
            "\"${__RagavanCommand}\" __run 'beta' \"${__RagavanOriginalCommands[1]}\" '0' \"$@\""
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
    fn home_maps_to_the_non_login_interactive_profile() {
        let home = if cfg!(windows) {
            Path::new("C:/Users/ragavan-test")
        } else {
            Path::new("/home/ragavan-test")
        };
        let bash = Bash::from_home(home).expect("absolute HOME should be accepted");

        assert_eq!(bash.profile, home.join(".bashrc"));
    }

    #[test]
    fn installation_renders_the_bash_bootstrap() {
        let directory = TestDirectory::new();
        let profile = directory.path().join(".bashrc");
        let original = b"alias serve=project\n";
        fs::write(&profile, original).expect("profile should be written");

        let edit = bash(&profile).install().expect("install should succeed");
        assert!(edit.changed);
        assert_eq!(edit.profile, profile);
        let installed = fs::read_to_string(&profile).expect("profile should remain UTF-8");
        assert!(installed.starts_with("alias serve=project\n"));
        assert!(installed.contains("builtin type -P ragavan"));
        assert!(installed.contains("ragavan hook bash"));
        assert!(!installed.contains("hook powershell"));

        assert!(
            bash(&profile)
                .uninstall()
                .expect("uninstall should succeed")
                .changed
        );
        assert_eq!(
            fs::read(&profile).expect("profile should be readable"),
            original
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

    fn bash(profile: &Path) -> Bash {
        Bash {
            profile: profile.to_owned(),
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
