use crate::presentation::{HumanOutput, Response};
use ragavan_shell::{InstallOutcome, UninstallOutcome};
use serde_json::{Map, Value as JsonValue, json};
use std::{io, path::PathBuf};

impl Response for InstallOutcome {
    fn write_human(&self, output: &mut HumanOutput<'_>) -> io::Result<()> {
        let shell = self.shell();
        output.success(format_args!(
            "Ragavan is {} for {}.",
            match self {
                Self::Installed { .. } => "installed",
                Self::AlreadyInstalled { .. } => "already installed",
            },
            shell.display_name()
        ))?;
        write_profiles(output, self.profiles())?;
        output.line(format_args!(
            "Future {} sessions will load Ragavan automatically.",
            shell.display_name()
        ))?;
        output.line(format_args!(
            "To activate Ragavan in an existing {} session, run `{}`.",
            shell.display_name(),
            shell.activation_command()
        ))
    }

    fn json_fields(&self) -> Map<String, JsonValue> {
        integration_fields(
            self.shell().name(),
            match self {
                Self::Installed { .. } => "installed",
                Self::AlreadyInstalled { .. } => "already_installed",
            },
            self.profiles(),
        )
    }
}

impl Response for UninstallOutcome {
    fn write_human(&self, output: &mut HumanOutput<'_>) -> io::Result<()> {
        let shell = self.shell();
        match self {
            Self::Uninstalled { .. } => {
                output.success(format_args!(
                    "Ragavan is uninstalled from {}.",
                    shell.display_name()
                ))?;
                write_profiles(output, self.profiles())?;
                output.line(format_args!(
                    "Future {} sessions will no longer load Ragavan automatically.",
                    shell.display_name()
                ))?;
                output.line(format_args!(
                    "Loaded integration remains active in existing sessions until they are closed."
                ))
            }
            Self::AlreadyUninstalled { .. } => output.success(format_args!(
                "Ragavan is already uninstalled from {}.",
                shell.display_name()
            )),
        }
    }

    fn json_fields(&self) -> Map<String, JsonValue> {
        integration_fields(
            self.shell().name(),
            match self {
                Self::Uninstalled { .. } => "uninstalled",
                Self::AlreadyUninstalled { .. } => "already_uninstalled",
            },
            self.profiles(),
        )
    }
}

fn write_profiles(output: &mut HumanOutput<'_>, profiles: &[PathBuf]) -> io::Result<()> {
    match profiles {
        [] => Ok(()),
        [profile] => output.field("Profile", format_args!("{}", profile.display())),
        profiles => {
            output.line(format_args!("Profiles:"))?;
            for profile in profiles {
                output.item(format_args!("{}", profile.display()))?;
            }
            Ok(())
        }
    }
}

fn integration_fields(shell: &str, state: &str, profiles: &[PathBuf]) -> Map<String, JsonValue> {
    Map::from_iter([(
        "integration".to_owned(),
        json!({
            "shell": shell,
            "state": state,
            "profiles": profiles
                .iter()
                .map(|profile| profile.to_string_lossy())
                .collect::<Vec<_>>(),
        }),
    )])
}
