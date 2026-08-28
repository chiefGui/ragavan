use crate::presentation::{HumanOutput, Response};
use ragavan_application::{Dashboard, LeaseState, RepositoryId, RepositoryState, WorktreeState};
use serde_json::{Map, Value as JsonValue, json};
use std::io;

impl Response for Dashboard {
    fn write_human(&self, output: &mut HumanOutput<'_>) -> io::Result<()> {
        if self.repositories().is_empty() {
            return output.line(format_args!("Ragavan is not managing any repositories."));
        }

        for (repository_index, repository) in self.repositories().iter().enumerate() {
            if repository_index != 0 {
                output.line(format_args!(""))?;
            }
            output.line(format_args!(
                "Repository {}",
                repository.id().map_or("unidentified", RepositoryId::as_str)
            ))?;
            output.line(format_args!(
                "  State: {}",
                repository_state(repository.state())
            ))?;
            if let Some(observed_id) = repository.observed_id() {
                output.line(format_args!("  Observed ID: {observed_id}"))?;
            }
            if let Some(registered_directory) = repository.registered_directory() {
                output.line(format_args!(
                    "  Registered directory: {}",
                    registered_directory.display()
                ))?;
            }
            if let Some(common_directory) = repository.common_directory() {
                output.line(format_args!(
                    "  Git directory: {}",
                    common_directory.display()
                ))?;
            }
            if repository.worktrees().is_empty() {
                output.line(format_args!("  Worktrees: none"))?;
                continue;
            }
            for worktree in repository.worktrees() {
                output.line(format_args!(
                    "  Worktree {}",
                    worktree.id().unwrap_or("unidentified")
                ))?;
                output.line(format_args!(
                    "    State: {}",
                    worktree_state(worktree.state())
                ))?;
                if let Some(path) = worktree.path() {
                    output.line(format_args!("    Path: {}", path.display()))?;
                }
                if worktree.services().is_empty() {
                    output.line(format_args!("    Services: none"))?;
                    continue;
                }
                for service in worktree.services() {
                    output.line(format_args!(
                        "    Service {}",
                        service.scope().unwrap_or("<root>")
                    ))?;
                    output.line(format_args!("      Port: {}", service.port()))?;
                    output.line(format_args!(
                        "      Lease: {}",
                        lease_state(service.lease())
                    ))?;
                }
            }
        }
        Ok(())
    }

    fn json_object(&self) -> Map<String, JsonValue> {
        let repositories = self
            .repositories()
            .iter()
            .map(|repository| {
                let worktrees = repository
                    .worktrees()
                    .iter()
                    .map(|worktree| {
                        let services = worktree
                            .services()
                            .iter()
                            .map(|service| {
                                json!({
                                    "scope": service.scope(),
                                    "port": service.port(),
                                    "lease": lease_state(service.lease()),
                                })
                            })
                            .collect::<Vec<_>>();
                        json!({
                            "id": worktree.id(),
                            "path": worktree.path().map(|path| path.to_string_lossy()),
                            "state": worktree_state(worktree.state()),
                            "services": services,
                        })
                    })
                    .collect::<Vec<_>>();
                let mut value = json!({
                    "id": repository.id().map(RepositoryId::as_str),
                    "common_directory": repository
                        .common_directory()
                        .map(|path| path.to_string_lossy()),
                    "state": repository_state(repository.state()),
                    "worktrees": worktrees,
                });
                if let Some(observed_id) = repository.observed_id() {
                    value["observed_id"] = JsonValue::from(observed_id.as_str());
                }
                if let Some(registered_directory) = repository.registered_directory() {
                    value["registered_directory"] =
                        JsonValue::from(registered_directory.to_string_lossy());
                }
                value
            })
            .collect::<Vec<_>>();
        Map::from_iter([("repositories".to_owned(), JsonValue::Array(repositories))])
    }
}

fn repository_state(state: RepositoryState) -> &'static str {
    match state {
        RepositoryState::Enabled => "enabled",
        RepositoryState::Disabled => "disabled",
        RepositoryState::Unavailable => "unavailable",
        RepositoryState::InvalidIdentity => "invalid_identity",
        RepositoryState::IdentityMismatch => "identity_mismatch",
        RepositoryState::Unregistered => "unregistered",
    }
}

fn worktree_state(state: WorktreeState) -> &'static str {
    match state {
        WorktreeState::Available => "available",
        WorktreeState::Unavailable => "unavailable",
    }
}

fn lease_state(state: LeaseState) -> &'static str {
    match state {
        LeaseState::Active => "active",
        LeaseState::Inactive => "inactive",
    }
}
