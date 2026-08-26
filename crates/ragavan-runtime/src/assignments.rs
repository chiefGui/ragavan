use super::{Error, PORT_RANGE_SIZE, PORT_RANGE_START};
use ragavan_core::ServiceIdentity;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const FILE_NAME: &str = "ports.json";
static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

type Entries = BTreeMap<String, (u64, u16)>;

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

pub(super) struct PortAssignments {
    path: PathBuf,
    entries: Entries,
}

impl PortAssignments {
    pub(super) fn read(state_directory: &Path) -> Result<Self, Error> {
        let path = state_directory.join(FILE_NAME);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self {
                    path,
                    entries: BTreeMap::new(),
                });
            }
            Err(source) => {
                return Err(Error::ReadAssignments { path, source });
            }
        };
        let entries: Entries =
            serde_json::from_slice(&bytes).map_err(|source| Error::ParseAssignments {
                path: path.clone(),
                source,
            })?;
        validate(&path, &entries)?;

        Ok(Self { path, entries })
    }

    pub(super) fn assignment(&self, identity: &ServiceIdentity) -> Result<Assignment, Error> {
        if let Some((slot, port)) = self.entries.get(&key(identity)).copied() {
            return Ok(Assignment {
                slot,
                port: Some(port),
            });
        }

        let slot = self
            .entries
            .values()
            .map(|(slot, _)| *slot)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| Error::InvalidAssignments {
                path: self.path.clone(),
                detail: "the service slot counter is exhausted".to_owned(),
            })?;
        Ok(Assignment { slot, port: None })
    }

    pub(super) fn remember(
        &mut self,
        identity: &ServiceIdentity,
        assignment: Assignment,
        port: u16,
    ) -> Result<(), Error> {
        let value = (assignment.slot, port);
        if self.entries.get(&key(identity)) == Some(&value) {
            return Ok(());
        }

        self.entries.insert(key(identity), value);
        self.write()
    }

    fn write(&self) -> Result<(), Error> {
        let bytes =
            serde_json::to_vec(&self.entries).map_err(|source| Error::SerializeAssignments {
                path: self.path.clone(),
                source,
            })?;
        let (temporary, mut file) = TemporaryFile::create(&self.path)?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| Error::WriteAssignments {
                path: self.path.clone(),
                source,
            })?;
        drop(file);
        fs::rename(temporary.path(), &self.path).map_err(|source| Error::WriteAssignments {
            path: self.path.clone(),
            source,
        })?;
        Ok(())
    }
}

fn key(identity: &ServiceIdentity) -> String {
    let worktree = identity.worktree();
    let scope = identity.scope().unwrap_or_default();

    format!(
        "s:{}:{}{}:{}{}:{}",
        worktree.repository_id().len(),
        worktree.repository_id(),
        worktree.worktree_id().len(),
        worktree.worktree_id(),
        scope.len(),
        scope
    )
}

fn validate(path: &Path, entries: &Entries) -> Result<(), Error> {
    let mut slots = BTreeSet::new();
    for (slot, port) in entries.values() {
        if *slot == 0 {
            return Err(Error::InvalidAssignments {
                path: path.to_owned(),
                detail: "a service has slot zero".to_owned(),
            });
        }
        if !slots.insert(*slot) {
            return Err(Error::InvalidAssignments {
                path: path.to_owned(),
                detail: format!("service slot {slot} is assigned more than once"),
            });
        }
        if !(PORT_RANGE_START..PORT_RANGE_START + PORT_RANGE_SIZE).contains(port) {
            return Err(Error::InvalidAssignments {
                path: path.to_owned(),
                detail: format!("port {port} is outside Ragavan's managed range"),
            });
        }
    }
    Ok(())
}

struct TemporaryFile(PathBuf);

impl TemporaryFile {
    fn create(destination: &Path) -> Result<(Self, File), Error> {
        let parent = destination
            .parent()
            .expect("Ragavan assignment files always have a parent directory");

        for _ in 0..100 {
            let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                ".{FILE_NAME}.{}-{sequence}.tmp",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((Self(path), file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(Error::WriteAssignments {
                        path: destination.to_owned(),
                        source,
                    });
                }
            }
        }

        Err(Error::WriteAssignments {
            path: destination.to_owned(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not allocate a temporary assignments file",
            ),
        })
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
