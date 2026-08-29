use crate::{Error, PORT_RANGE_SIZE, PORT_RANGE_START};
use ragavan_core::{RepositoryId, ServiceIdentity, ServiceScope, WorktreeIdentity};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) const FILE_NAME: &str = "state.json";
static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub(super) struct Assignment {
    slot: u64,
    port: Option<u16>,
}

impl Assignment {
    pub(super) fn slot(self) -> u64 {
        self.slot
    }

    pub(super) fn port(self) -> Option<u16> {
        self.port
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct ServiceRecord {
    slot: u64,
    port: u16,
}

pub(super) struct State {
    path: PathBuf,
    repositories: BTreeMap<RepositoryId, PathBuf>,
    services: BTreeMap<ServiceIdentity, ServiceRecord>,
}

impl State {
    pub(super) fn read(state_directory: &Path) -> Result<Self, Error> {
        let path = state_directory.join(FILE_NAME);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    path,
                    repositories: BTreeMap::new(),
                    services: BTreeMap::new(),
                });
            }
            Err(source) => return Err(Error::ReadState { path, source }),
        };
        let stored: StoredState =
            serde_json::from_slice(&bytes).map_err(|source| Error::ParseState {
                path: path.clone(),
                source,
            })?;
        Self::from_stored(path, stored)
    }

    pub(super) fn repositories(&self) -> &BTreeMap<RepositoryId, PathBuf> {
        &self.repositories
    }

    pub(super) fn services(&self) -> impl Iterator<Item = (&ServiceIdentity, u64, u16)> + '_ {
        self.services
            .iter()
            .map(|(identity, service)| (identity, service.slot, service.port))
    }

    pub(super) fn assignment(&self, identity: &ServiceIdentity) -> Result<Assignment, Error> {
        if let Some(service) = self.services.get(identity).copied() {
            return Ok(Assignment {
                slot: service.slot,
                port: Some(service.port),
            });
        }

        let slot = self
            .services
            .values()
            .map(|service| service.slot)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| self.invalid("the service slot counter is exhausted"))?;
        Ok(Assignment { slot, port: None })
    }

    pub(super) fn remember(
        &mut self,
        identity: &ServiceIdentity,
        assignment: Assignment,
        port: u16,
    ) -> Result<(), Error> {
        let value = ServiceRecord {
            slot: assignment.slot,
            port,
        };
        if self.services.get(identity) == Some(&value) {
            return Ok(());
        }
        self.services.insert(identity.clone(), value);
        self.write()
    }

    pub(super) fn register(
        &mut self,
        repository_id: &RepositoryId,
        common_directory: &Path,
    ) -> Result<(), Error> {
        if !common_directory.is_absolute() {
            return Err(Error::InvalidRepositoryDirectory {
                path: common_directory.to_owned(),
            });
        }
        let canonical_current = canonicalize(common_directory)?;
        let mut changed = false;

        if let Some(registered) = self.repositories.get(repository_id) {
            match canonicalize_if_present(registered)? {
                Some(canonical_registered) if canonical_registered != canonical_current => {
                    return Err(Error::RepositoryIdentityConflict {
                        repository_id: repository_id.clone(),
                        registered_directory: registered.clone(),
                        current_directory: common_directory.to_owned(),
                    });
                }
                Some(_) => {}
                None => changed = true,
            }
        } else {
            changed = true;
        }

        let stale_ids = self
            .repositories
            .iter()
            .filter_map(|(id, directory)| {
                if id == repository_id {
                    return None;
                }
                match canonicalize_if_present(directory) {
                    Ok(Some(canonical)) if canonical == canonical_current => Some(Ok(id.clone())),
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        for stale_id in stale_ids {
            self.repositories.remove(&stale_id);
            changed = true;
        }

        if self.repositories.get(repository_id).map(PathBuf::as_path) != Some(common_directory) {
            self.repositories
                .insert(repository_id.clone(), common_directory.to_owned());
            changed = true;
        }

        if changed { self.write() } else { Ok(()) }
    }

    pub(super) fn unregister(
        &mut self,
        repository_id: Option<&RepositoryId>,
        common_directory: &Path,
    ) -> Result<(), Error> {
        let canonical_current = canonicalize(common_directory)?;
        let remove = self
            .repositories
            .iter()
            .filter_map(|(registered_id, registered_directory)| {
                let same_identity = repository_id == Some(registered_id);
                match canonicalize_if_present(registered_directory) {
                    Ok(Some(canonical_registered)) if canonical_registered == canonical_current => {
                        Some(Ok(registered_id.clone()))
                    }
                    Ok(None) if same_identity => Some(Ok(registered_id.clone())),
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        if remove.is_empty() {
            return Ok(());
        }

        for repository_id in remove {
            self.repositories.remove(&repository_id);
        }
        self.write()
    }

    fn from_stored(path: PathBuf, stored: StoredState) -> Result<Self, Error> {
        let mut repositories = BTreeMap::new();
        let mut directories = BTreeSet::new();
        for repository in stored.repositories {
            let repository_id = RepositoryId::new(repository.id)
                .map_err(|error| invalid(&path, error.to_string()))?;
            if !repository.common_directory.is_absolute() {
                return Err(invalid(
                    &path,
                    format!("repository `{repository_id}` has a non-absolute common directory"),
                ));
            }
            if !directories.insert(repository.common_directory.clone()) {
                return Err(invalid(
                    &path,
                    format!(
                        "common directory `{}` is registered more than once",
                        repository.common_directory.display()
                    ),
                ));
            }
            if repositories
                .insert(repository_id.clone(), repository.common_directory)
                .is_some()
            {
                return Err(invalid(
                    &path,
                    format!("repository `{repository_id}` is registered more than once"),
                ));
            }
        }

        let mut services = BTreeMap::new();
        let mut slots = BTreeSet::new();
        for service in stored.services {
            if service.slot == 0 {
                return Err(invalid(&path, "a service has slot zero"));
            }
            if !slots.insert(service.slot) {
                return Err(invalid(
                    &path,
                    format!("service slot {} is assigned more than once", service.slot),
                ));
            }
            if !(PORT_RANGE_START..PORT_RANGE_START + PORT_RANGE_SIZE).contains(&service.port) {
                return Err(invalid(
                    &path,
                    format!("port {} is outside Ragavan's managed range", service.port),
                ));
            }

            let repository_id = RepositoryId::new(service.repository_id)
                .map_err(|error| invalid(&path, error.to_string()))?;
            let worktree = WorktreeIdentity::new(repository_id, service.worktree_id)
                .map_err(|error| invalid(&path, error.to_string()))?;
            let scope = ServiceScope::from_relative_path(Path::new(
                service.scope.as_deref().unwrap_or_default(),
            ))
            .map_err(|error| invalid(&path, error.to_string()))?;
            if scope.relative_path() != service.scope.as_deref() {
                return Err(invalid(&path, "a service scope is not canonical"));
            }
            let identity = ServiceIdentity::new(worktree, scope);
            if services
                .insert(
                    identity,
                    ServiceRecord {
                        slot: service.slot,
                        port: service.port,
                    },
                )
                .is_some()
            {
                return Err(invalid(
                    &path,
                    "a service identity is assigned more than once",
                ));
            }
        }

        Ok(Self {
            path,
            repositories,
            services,
        })
    }

    fn write(&self) -> Result<(), Error> {
        let stored = StoredState {
            repositories: self
                .repositories
                .iter()
                .map(|(repository_id, common_directory)| StoredRepository {
                    id: repository_id.as_str().to_owned(),
                    common_directory: common_directory.clone(),
                })
                .collect(),
            services: self
                .services
                .iter()
                .map(|(identity, service)| StoredService {
                    repository_id: identity.worktree().repository_id().as_str().to_owned(),
                    worktree_id: identity.worktree().worktree_id().to_owned(),
                    scope: identity.scope().relative_path().map(str::to_owned),
                    slot: service.slot,
                    port: service.port,
                })
                .collect(),
        };
        let bytes = serde_json::to_vec(&stored).map_err(|source| Error::SerializeState {
            path: self.path.clone(),
            source,
        })?;
        write_atomically(&self.path, &bytes).map_err(|source| Error::WriteState {
            path: self.path.clone(),
            source,
        })
    }

    fn invalid(&self, detail: impl Into<String>) -> Error {
        invalid(&self.path, detail)
    }
}

pub(super) fn conflicts_with_live_directory(
    registered: &Path,
    current: &Path,
) -> Result<bool, Error> {
    let Some(canonical_registered) = canonicalize_if_present(registered)? else {
        return Ok(false);
    };
    Ok(canonical_registered != canonicalize(current)?)
}

pub(super) fn is_same_live_directory(registered: &Path, current: &Path) -> Result<bool, Error> {
    let Some(canonical_registered) = canonicalize_if_present(registered)? else {
        return Ok(false);
    };
    Ok(canonical_registered == canonicalize(current)?)
}

fn invalid(path: &Path, detail: impl Into<String>) -> Error {
    Error::InvalidState {
        path: path.to_owned(),
        detail: detail.into(),
    }
}

fn canonicalize(path: &Path) -> Result<PathBuf, Error> {
    fs::canonicalize(path).map_err(|source| Error::InspectRepositoryDirectory {
        path: path.to_owned(),
        source,
    })
}

fn canonicalize_if_present(path: &Path) -> Result<Option<PathBuf>, Error> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(Some(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::InspectRepositoryDirectory {
            path: path.to_owned(),
            source,
        }),
    }
}

fn write_atomically(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let (temporary, mut file) = TemporaryFile::create(destination)?;
    file.write_all(bytes).and_then(|()| file.sync_all())?;
    drop(file);
    fs::rename(temporary.path(), destination)
}

struct TemporaryFile(PathBuf);

impl TemporaryFile {
    fn create(destination: &Path) -> io::Result<(Self, File)> {
        let parent = destination
            .parent()
            .expect("Ragavan state files always have a parent directory");

        for _ in 0..100 {
            let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                ".{FILE_NAME}.{}-{sequence}.tmp",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((Self(path), file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }

        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a temporary state file",
        ))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        match fs::remove_file(&self.0) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => {}
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredState {
    repositories: Vec<StoredRepository>,
    services: Vec<StoredService>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRepository {
    id: String,
    common_directory: PathBuf,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredService {
    repository_id: String,
    worktree_id: String,
    scope: Option<String>,
    slot: u64,
    port: u16,
}
