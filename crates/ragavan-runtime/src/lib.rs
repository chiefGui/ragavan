#![forbid(unsafe_code)]

mod state;

use self::state::State;
use ragavan_core::{LeaseState, Port, RepositoryId, ServiceIdentity};
use ragavan_diagnostics::{Detail, Diagnostic};
use std::{
    env,
    ffi::OsString,
    fmt, fs,
    fs::{File, OpenOptions, TryLockError},
    io,
    net::{TcpListener, ToSocketAddrs},
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

const PORT_RANGE_START: u16 = 10_000;
const PORT_RANGE_SIZE: u16 = 20_000;
const STATE_LOCK_FILE: &str = "state.lock";
const SERVICE_LOCKS_DIRECTORY: &str = "services";
const PORT_LOCKS_DIRECTORY: &str = "ports";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryRegistration {
    repository_id: RepositoryId,
    common_directory: PathBuf,
}

impl RepositoryRegistration {
    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn common_directory(&self) -> &Path {
        &self.common_directory
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceAssignment {
    identity: ServiceIdentity,
    port: Port,
    lease: LeaseState,
}

impl ServiceAssignment {
    pub fn identity(&self) -> &ServiceIdentity {
        &self.identity
    }

    pub fn port(&self) -> Port {
        self.port
    }

    pub fn lease(&self) -> LeaseState {
        self.lease
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeSnapshot {
    repositories: Vec<RepositoryRegistration>,
    services: Vec<ServiceAssignment>,
}

impl RuntimeSnapshot {
    pub fn repositories(&self) -> &[RepositoryRegistration] {
        &self.repositories
    }

    pub fn services(&self) -> &[ServiceAssignment] {
        &self.services
    }

    /// Find the registration that resolves to the given live Git directory.
    pub fn registration_for_directory(
        &self,
        current_directory: &Path,
    ) -> Result<Option<&RepositoryRegistration>, Error> {
        for registration in &self.repositories {
            if state::is_same_live_directory(registration.common_directory(), current_directory)? {
                return Ok(Some(registration));
            }
        }
        Ok(None)
    }

    /// Find a different live directory registered under the given repository identity.
    pub fn conflicting_repository_directory(
        &self,
        repository_id: &RepositoryId,
        current_directory: &Path,
    ) -> Result<Option<&Path>, Error> {
        let Some(registration) = self
            .repositories
            .iter()
            .find(|registration| registration.repository_id() == repository_id)
        else {
            return Ok(None);
        };
        if state::conflicts_with_live_directory(registration.common_directory(), current_directory)?
        {
            Ok(Some(registration.common_directory()))
        } else {
            Ok(None)
        }
    }
}

/// An exclusive service port held for the lifetime of a development process.
pub struct PortLease {
    port: Port,
    service_lock: File,
    port_lock: File,
    reservations: Vec<TcpListener>,
}

impl PortLease {
    pub fn port(&self) -> Port {
        self.port
    }

    /// Release the socket reservation immediately before starting the process,
    /// then retain both allocation locks until the process has stopped.
    pub fn run(self, command: &mut Command) -> Result<ExitStatus, Error> {
        let executable = command.get_program().to_owned();
        let Self {
            port: _,
            service_lock,
            port_lock,
            reservations,
        } = self;

        drop(reservations);
        let result = command
            .status()
            .map_err(|source| Error::RunProcess { executable, source });
        drop(port_lock);
        drop(service_lock);
        result
    }
}

/// Acquire the stable available port for one service.
pub fn acquire_port(identity: &ServiceIdentity) -> Result<PortLease, Error> {
    acquire_port_in(&state_directory()?, identity)
}

/// Make a Git repository discoverable by global Ragavan commands.
pub fn register_repository(
    repository_id: &RepositoryId,
    common_directory: &Path,
) -> Result<(), Error> {
    register_repository_in(&state_directory()?, repository_id, common_directory)
}

fn register_repository_in(
    state_directory: &Path,
    repository_id: &RepositoryId,
    common_directory: &Path,
) -> Result<(), Error> {
    let _state_lock = lock_state_for_mutation(state_directory)?;
    let mut state = State::read(state_directory)?;
    state.register(repository_id, common_directory)
}

/// Remove a repository from global discovery without deleting its port history.
pub fn unregister_repository(
    repository_id: Option<&RepositoryId>,
    common_directory: &Path,
) -> Result<(), Error> {
    unregister_repository_in(&state_directory()?, repository_id, common_directory)
}

fn unregister_repository_in(
    state_directory: &Path,
    repository_id: Option<&RepositoryId>,
    common_directory: &Path,
) -> Result<(), Error> {
    let _state_lock = lock_state_for_mutation(state_directory)?;
    let mut state = State::read(state_directory)?;
    state.unregister(repository_id, common_directory)
}

/// Read a consistent, one-shot view of Ragavan's local repository and service state.
pub fn snapshot() -> Result<RuntimeSnapshot, Error> {
    snapshot_in(&state_directory()?)
}

fn snapshot_in(state_directory: &Path) -> Result<RuntimeSnapshot, Error> {
    match fs::metadata(state_directory) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(Error::InvalidStateDirectory {
                path: state_directory.to_owned(),
            });
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(RuntimeSnapshot::default());
        }
        Err(source) => {
            return Err(Error::InspectState {
                path: state_directory.to_owned(),
                source,
            });
        }
    }

    if !state_file_exists(state_directory)? {
        return Ok(RuntimeSnapshot::default());
    }

    let lock_path = state_directory.join(STATE_LOCK_FILE);
    let state_lock = open_existing_lock(&lock_path)?.ok_or_else(|| Error::MissingStateLock {
        path: lock_path.clone(),
    })?;
    state_lock.lock().map_err(|source| Error::LockState {
        path: lock_path,
        source,
    })?;

    let state = State::read(state_directory)?;
    let repositories = state
        .repositories()
        .iter()
        .map(|(repository_id, common_directory)| RepositoryRegistration {
            repository_id: repository_id.clone(),
            common_directory: common_directory.clone(),
        })
        .collect();
    let service_locks = state_directory.join(SERVICE_LOCKS_DIRECTORY);
    let mut services = Vec::new();
    for (identity, slot, port) in state.services() {
        let service_lock_path = service_locks.join(format!("{slot}.lock"));
        let lease = match open_existing_lock(&service_lock_path)? {
            None => LeaseState::Inactive,
            Some(lock) => match lock.try_lock() {
                Ok(()) => LeaseState::Inactive,
                Err(TryLockError::WouldBlock) => LeaseState::Active,
                Err(TryLockError::Error(source)) => {
                    return Err(Error::LockService {
                        path: service_lock_path,
                        source,
                    });
                }
            },
        };
        services.push(ServiceAssignment {
            identity: identity.clone(),
            port: Port::new(port).expect("validated Ragavan state excludes port zero"),
            lease,
        });
    }

    drop(state_lock);
    Ok(RuntimeSnapshot {
        repositories,
        services,
    })
}

fn acquire_port_in(state_directory: &Path, identity: &ServiceIdentity) -> Result<PortLease, Error> {
    let service_locks = state_directory.join(SERVICE_LOCKS_DIRECTORY);
    let port_locks = state_directory.join(PORT_LOCKS_DIRECTORY);
    for directory in [state_directory, &service_locks, &port_locks] {
        fs::create_dir_all(directory).map_err(|source| Error::CreateState {
            path: directory.to_owned(),
            source,
        })?;
    }

    let state_lock = lock_state_for_mutation(state_directory)?;

    let mut state = State::read(state_directory)?;
    let assignment = state.assignment(identity)?;

    let service_lock_path = service_locks.join(format!("{}.lock", assignment.slot()));
    let service_lock = open_lock(&service_lock_path)?;
    match service_lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(Error::ServiceAlreadyRunning),
        Err(TryLockError::Error(source)) => {
            return Err(Error::LockService {
                path: service_lock_path,
                source,
            });
        }
    }

    let (port, port_lock, reservations) = reserve_port(&port_locks, identity, assignment.port())?;
    state.remember(identity, assignment, port.get())?;

    drop(state_lock);
    Ok(PortLease {
        port,
        service_lock,
        port_lock,
        reservations,
    })
}

fn reserve_port(
    port_locks: &Path,
    identity: &ServiceIdentity,
    preferred_port: Option<u16>,
) -> Result<(Port, File, Vec<TcpListener>), Error> {
    if let Some(port) = preferred_port
        && let Some(reservation) = try_reserve_port(port_locks, port)?
    {
        return Ok(reservation);
    }

    let start = preferred_port_for(identity).get() - PORT_RANGE_START;
    for offset in 0..PORT_RANGE_SIZE {
        let port = PORT_RANGE_START + (start + offset) % PORT_RANGE_SIZE;
        if preferred_port == Some(port) {
            continue;
        }
        if let Some(reservation) = try_reserve_port(port_locks, port)? {
            return Ok(reservation);
        }
    }

    Err(Error::NoAvailablePort)
}

fn try_reserve_port(
    port_locks: &Path,
    value: u16,
) -> Result<Option<(Port, File, Vec<TcpListener>)>, Error> {
    let lock_path = port_locks.join(format!("{value}.lock"));
    let lock = open_lock(&lock_path)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(TryLockError::Error(source)) => {
            return Err(Error::LockPort {
                path: lock_path,
                source,
            });
        }
    }

    let mut addresses: Vec<_> = ("localhost", value)
        .to_socket_addrs()
        .map_err(|source| Error::CheckPort {
            port: value,
            source,
        })?
        .collect();
    if addresses.is_empty() {
        return Err(Error::ResolveLocalhost { port: value });
    }
    addresses.sort_unstable();
    addresses.dedup();

    let mut reservations = Vec::with_capacity(addresses.len());
    for address in addresses {
        match TcpListener::bind(address) {
            Ok(reservation) => reservations.push(reservation),
            Err(source) if source.kind() == io::ErrorKind::AddrInUse => return Ok(None),
            Err(source) => {
                return Err(Error::CheckPort {
                    port: value,
                    source,
                });
            }
        }
    }

    Ok(Some((
        Port::new(value).expect("Ragavan's port range excludes zero"),
        lock,
        reservations,
    )))
}

fn preferred_port_for(identity: &ServiceIdentity) -> Port {
    let worktree = identity.worktree();
    let range_size = u64::from(PORT_RANGE_SIZE);
    let repository_slot = stable_hash(worktree.repository_id().as_str()) % range_size;
    let worktree_slot = stable_hash(worktree.worktree_id()) % range_size;
    let service_slot = identity
        .scope()
        .relative_path()
        .map_or(0, |scope| stable_hash(scope) % range_size);
    let value =
        PORT_RANGE_START + ((repository_slot + worktree_slot + service_slot) % range_size) as u16;

    Port::new(value).expect("Ragavan's port range excludes zero")
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

fn open_lock(path: &Path) -> Result<File, Error> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|source| Error::OpenLock {
            path: path.to_owned(),
            source,
        })
}

fn open_existing_lock(path: &Path) -> Result<Option<File>, Error> {
    match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::OpenLock {
            path: path.to_owned(),
            source,
        }),
    }
}

fn lock_state_for_mutation(state_directory: &Path) -> Result<File, Error> {
    fs::create_dir_all(state_directory).map_err(|source| Error::CreateState {
        path: state_directory.to_owned(),
        source,
    })?;
    let lock_path = state_directory.join(STATE_LOCK_FILE);
    let lock = open_lock(&lock_path)?;
    lock.lock().map_err(|source| Error::LockState {
        path: lock_path,
        source,
    })?;
    Ok(lock)
}

fn state_file_exists(state_directory: &Path) -> Result<bool, Error> {
    let path = state_directory.join(state::FILE_NAME);
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err(Error::InvalidStateFile { path }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(Error::InspectState { path, source }),
    }
}

fn state_directory() -> Result<PathBuf, Error> {
    #[cfg(windows)]
    let (variable, home) = ("LOCALAPPDATA", env::var_os("LOCALAPPDATA"));
    #[cfg(not(windows))]
    let (variable, home) = match env::var_os("XDG_STATE_HOME") {
        Some(home) if !home.is_empty() => ("XDG_STATE_HOME", Some(home)),
        _ => (
            "HOME",
            env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state").into()),
        ),
    };

    let home = home
        .filter(|home| !home.is_empty())
        .ok_or(Error::StateHomeUnavailable { variable })?;
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        return Err(Error::InvalidStateHome {
            variable,
            path: home,
        });
    }

    Ok(home.join("ragavan"))
}

#[derive(Debug)]
pub enum Error {
    StateHomeUnavailable {
        variable: &'static str,
    },
    InvalidStateHome {
        variable: &'static str,
        path: PathBuf,
    },
    InvalidStateDirectory {
        path: PathBuf,
    },
    InvalidStateFile {
        path: PathBuf,
    },
    InspectState {
        path: PathBuf,
        source: io::Error,
    },
    CreateState {
        path: PathBuf,
        source: io::Error,
    },
    OpenLock {
        path: PathBuf,
        source: io::Error,
    },
    LockState {
        path: PathBuf,
        source: io::Error,
    },
    MissingStateLock {
        path: PathBuf,
    },
    LockService {
        path: PathBuf,
        source: io::Error,
    },
    LockPort {
        path: PathBuf,
        source: io::Error,
    },
    ServiceAlreadyRunning,
    ReadState {
        path: PathBuf,
        source: io::Error,
    },
    ParseState {
        path: PathBuf,
        source: serde_json::Error,
    },
    SerializeState {
        path: PathBuf,
        source: serde_json::Error,
    },
    InvalidState {
        path: PathBuf,
        detail: String,
    },
    WriteState {
        path: PathBuf,
        source: io::Error,
    },
    InvalidRepositoryDirectory {
        path: PathBuf,
    },
    InspectRepositoryDirectory {
        path: PathBuf,
        source: io::Error,
    },
    RepositoryIdentityConflict {
        repository_id: RepositoryId,
        registered_directory: PathBuf,
        current_directory: PathBuf,
    },
    ResolveLocalhost {
        port: u16,
    },
    CheckPort {
        port: u16,
        source: io::Error,
    },
    NoAvailablePort,
    RunProcess {
        executable: OsString,
        source: io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StateHomeUnavailable { variable } => write!(
                formatter,
                "could not locate Ragavan's local state because {variable} is unavailable"
            ),
            Self::InvalidStateHome { variable, path } => write!(
                formatter,
                "could not use {variable} for Ragavan's local state because {} is not an absolute path",
                path.display()
            ),
            Self::InvalidStateDirectory { path } => write!(
                formatter,
                "could not use Ragavan state path {} because it is not a directory",
                path.display()
            ),
            Self::InvalidStateFile { path } => write!(
                formatter,
                "could not use Ragavan state file {} because it is not a regular file",
                path.display()
            ),
            Self::InspectState { path, source } => write!(
                formatter,
                "could not inspect Ragavan state at {}: {source}",
                path.display()
            ),
            Self::CreateState { path, source } => write!(
                formatter,
                "could not create Ragavan state directory {}: {source}",
                path.display()
            ),
            Self::OpenLock { path, source } => write!(
                formatter,
                "could not open Ragavan lock {}: {source}",
                path.display()
            ),
            Self::LockState { path, source } => write!(
                formatter,
                "could not lock Ragavan's local state at {}: {source}",
                path.display()
            ),
            Self::MissingStateLock { path } => write!(
                formatter,
                "could not read Ragavan state because its coordination lock is missing at {}",
                path.display()
            ),
            Self::LockService { path, source } => write!(
                formatter,
                "could not lock the service at {}: {source}",
                path.display()
            ),
            Self::LockPort { path, source } => write!(
                formatter,
                "could not lock a service port at {}: {source}",
                path.display()
            ),
            Self::ServiceAlreadyRunning => formatter.write_str(
                "could not start the development process: this service already has an active development process",
            ),
            Self::ReadState { path, source } => write!(
                formatter,
                "could not read Ragavan state from {}: {source}",
                path.display()
            ),
            Self::ParseState { path, source } => write!(
                formatter,
                "could not parse Ragavan state at {}: {source}",
                path.display()
            ),
            Self::SerializeState { path, source } => write!(
                formatter,
                "could not serialize Ragavan state for {}: {source}",
                path.display()
            ),
            Self::InvalidState { path, detail } => write!(
                formatter,
                "could not use Ragavan state at {}: {detail}",
                path.display()
            ),
            Self::WriteState { path, source } => write!(
                formatter,
                "could not write Ragavan state to {}: {source}",
                path.display()
            ),
            Self::InvalidRepositoryDirectory { path } => write!(
                formatter,
                "could not register Git directory {} because it is not an absolute path",
                path.display()
            ),
            Self::InspectRepositoryDirectory { path, source } => write!(
                formatter,
                "could not inspect registered Git directory {}: {source}",
                path.display()
            ),
            Self::RepositoryIdentityConflict {
                repository_id,
                registered_directory,
                current_directory,
            } => write!(
                formatter,
                "repository identity {repository_id} is already registered to {}, not {}",
                registered_directory.display(),
                current_directory.display()
            ),
            Self::ResolveLocalhost { port } => write!(
                formatter,
                "could not resolve localhost while checking port {port}"
            ),
            Self::CheckPort { port, source } => {
                write!(formatter, "could not check local port {port}: {source}")
            }
            Self::NoAvailablePort => formatter.write_str(
                "could not start the development process: no local port is available in Ragavan's managed range 10000-29999",
            ),
            Self::RunProcess { executable, source } => write!(
                formatter,
                "could not run {}: {source}",
                Path::new(executable).display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CreateState { source, .. }
            | Self::InspectState { source, .. }
            | Self::OpenLock { source, .. }
            | Self::LockState { source, .. }
            | Self::LockService { source, .. }
            | Self::LockPort { source, .. }
            | Self::ReadState { source, .. }
            | Self::WriteState { source, .. }
            | Self::InspectRepositoryDirectory { source, .. }
            | Self::CheckPort { source, .. }
            | Self::RunProcess { source, .. } => Some(source),
            Self::ParseState { source, .. } | Self::SerializeState { source, .. } => Some(source),
            Self::StateHomeUnavailable { .. }
            | Self::InvalidStateHome { .. }
            | Self::InvalidStateDirectory { .. }
            | Self::InvalidStateFile { .. }
            | Self::MissingStateLock { .. }
            | Self::ServiceAlreadyRunning
            | Self::InvalidState { .. }
            | Self::InvalidRepositoryDirectory { .. }
            | Self::RepositoryIdentityConflict { .. }
            | Self::ResolveLocalhost { .. }
            | Self::NoAvailablePort => None,
        }
    }
}

impl Diagnostic for Error {
    fn code(&self) -> &'static str {
        match self {
            Self::StateHomeUnavailable { .. } => "runtime.state_home.unavailable",
            Self::InvalidStateHome { .. } => "runtime.state_home.invalid",
            Self::InvalidStateDirectory { .. } => "runtime.state_directory.invalid",
            Self::InvalidStateFile { .. } => "runtime.state_file.invalid",
            Self::InspectState { .. } => "runtime.state.inspect",
            Self::CreateState { .. } => "runtime.state.create",
            Self::OpenLock { .. } => "runtime.lock.open",
            Self::LockState { .. } => "runtime.lock.state",
            Self::MissingStateLock { .. } => "runtime.lock.missing",
            Self::LockService { .. } => "runtime.lock.service",
            Self::LockPort { .. } => "runtime.lock.port",
            Self::ServiceAlreadyRunning => "runtime.service.already_running",
            Self::ReadState { .. } => "runtime.state.read",
            Self::ParseState { .. } => "runtime.state.parse",
            Self::SerializeState { .. } => "runtime.state.serialize",
            Self::InvalidState { .. } => "runtime.state.invalid",
            Self::WriteState { .. } => "runtime.state.write",
            Self::InvalidRepositoryDirectory { .. } => "runtime.repository_directory.invalid",
            Self::InspectRepositoryDirectory { .. } => "runtime.repository_directory.inspect",
            Self::RepositoryIdentityConflict { .. } => "runtime.repository_identity.conflict",
            Self::ResolveLocalhost { .. } => "runtime.localhost.resolve",
            Self::CheckPort { .. } => "runtime.port.check",
            Self::NoAvailablePort => "runtime.port.unavailable",
            Self::RunProcess { .. } => "runtime.process.start",
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            Self::StateHomeUnavailable { variable } => {
                Some(format!("set {variable} to an absolute local directory"))
            }
            Self::InvalidStateHome { variable, .. } => {
                Some(format!("set {variable} to an absolute path"))
            }
            Self::ServiceAlreadyRunning => Some(
                "stop the existing development process for this service, then retry".to_owned(),
            ),
            Self::RepositoryIdentityConflict { .. } => Some(
                "disable Ragavan in the copied repository, then enable it again to assign a new identity"
                    .to_owned(),
            ),
            Self::NoAvailablePort => Some(format!(
                "free a local port in {}-{} and retry",
                PORT_RANGE_START,
                PORT_RANGE_START + PORT_RANGE_SIZE - 1
            )),
            _ => None,
        }
    }

    fn details(&self) -> Vec<Detail> {
        match self {
            Self::StateHomeUnavailable { variable } => {
                vec![Detail::text("variable", *variable)]
            }
            Self::InvalidStateHome { variable, path } => vec![
                Detail::text("variable", *variable),
                Detail::text("path", path.display().to_string()),
            ],
            Self::CreateState { path, .. }
            | Self::InvalidStateDirectory { path }
            | Self::InvalidStateFile { path }
            | Self::InspectState { path, .. }
            | Self::OpenLock { path, .. }
            | Self::LockState { path, .. }
            | Self::MissingStateLock { path }
            | Self::LockService { path, .. }
            | Self::LockPort { path, .. }
            | Self::ReadState { path, .. }
            | Self::ParseState { path, .. }
            | Self::SerializeState { path, .. }
            | Self::WriteState { path, .. }
            | Self::InvalidRepositoryDirectory { path }
            | Self::InspectRepositoryDirectory { path, .. } => {
                vec![Detail::text("path", path.display().to_string())]
            }
            Self::InvalidState { path, detail } => vec![
                Detail::text("path", path.display().to_string()),
                Detail::text("reason", detail),
            ],
            Self::RepositoryIdentityConflict {
                repository_id,
                registered_directory,
                current_directory,
            } => vec![
                Detail::text("repository_id", repository_id.as_str()),
                Detail::text(
                    "registered_directory",
                    registered_directory.display().to_string(),
                ),
                Detail::text("current_directory", current_directory.display().to_string()),
            ],
            Self::ResolveLocalhost { port } | Self::CheckPort { port, .. } => {
                vec![Detail::number("port", u64::from(*port))]
            }
            Self::NoAvailablePort => vec![
                Detail::number("range_start", u64::from(PORT_RANGE_START)),
                Detail::number(
                    "range_end",
                    u64::from(PORT_RANGE_START + PORT_RANGE_SIZE - 1),
                ),
            ],
            Self::RunProcess { executable, .. } => vec![Detail::text(
                "executable",
                Path::new(executable).display().to_string(),
            )],
            Self::ServiceAlreadyRunning => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Error, LeaseState, acquire_port_in, register_repository_in, snapshot_in,
        unregister_repository_in,
    };
    use ragavan_core::{RepositoryId, ServiceIdentity, ServiceScope, WorktreeIdentity};
    use std::{
        collections::BTreeSet,
        fs, io,
        net::TcpListener,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn a_port_is_stable_and_reserved_for_the_lease_lifetime() {
        let state = TestDirectory::new();
        let lease = acquire_port_in(state.path(), &scoped_service("stable", "apps/web"))
            .expect("port should be acquired");
        let port = lease.port().get();
        assert!(TcpListener::bind(("localhost", port)).is_err());

        drop(lease);
        let reservation = TcpListener::bind(("localhost", port))
            .expect("dropping the lease should release its socket reservation");
        drop(reservation);

        let lease = acquire_port_in(state.path(), &scoped_service("stable", "apps/web"))
            .expect("port should be reacquired");
        assert_eq!(lease.port().get(), port);
    }

    #[test]
    fn an_occupied_preferred_port_is_reassigned_stably() {
        let state = TestDirectory::new();
        let preferred = acquire_port_in(state.path(), &scoped_service("occupied", "apps/web"))
            .expect("preferred port should be acquired")
            .port()
            .get();
        let occupied = TcpListener::bind(("localhost", preferred))
            .expect("the preferred port should be free after the lease is dropped");

        let reassigned = acquire_port_in(state.path(), &scoped_service("occupied", "apps/web"))
            .expect("another port should be acquired")
            .port()
            .get();
        assert_ne!(reassigned, preferred);
        drop(occupied);

        let lease = acquire_port_in(state.path(), &scoped_service("occupied", "apps/web"))
            .expect("the reassigned port should be reacquired");
        assert_eq!(lease.port().get(), reassigned);
    }

    #[test]
    fn simultaneous_worktrees_receive_distinct_ports() {
        let state = TestDirectory::new();
        let first = acquire_port_in(state.path(), &scoped_service("worktree-1189", "apps/web"))
            .expect("first port should be acquired");
        let second = acquire_port_in(state.path(), &scoped_service("worktree-1754", "apps/web"))
            .expect("second port should be acquired");

        assert_ne!(first.port(), second.port());
    }

    #[test]
    fn simultaneous_repositories_receive_distinct_ports() {
        let state = TestDirectory::new();
        let first = acquire_port_in(
            state.path(),
            &scoped_service_in("repository-a", "same-worktree", "apps/web"),
        )
        .expect("first port should be acquired");
        let second = acquire_port_in(
            state.path(),
            &scoped_service_in("repository-b", "same-worktree", "apps/web"),
        )
        .expect("second port should be acquired");

        assert_ne!(first.port(), second.port());
    }

    #[test]
    fn one_service_cannot_start_twice() {
        let state = TestDirectory::new();
        let _lease = acquire_port_in(state.path(), &scoped_service("same", "apps/web"))
            .expect("port should be acquired");

        let error = acquire_port_in(state.path(), &scoped_service("same", "apps/web"))
            .err()
            .expect("a second lease should be rejected");
        assert!(matches!(error, Error::ServiceAlreadyRunning));
    }

    #[test]
    fn simultaneous_services_in_one_worktree_receive_distinct_ports() {
        let state = TestDirectory::new();
        let root = acquire_port_in(state.path(), &root_service("multi-service"))
            .expect("the root service should acquire a port");
        let web = acquire_port_in(state.path(), &scoped_service("multi-service", "apps/web"))
            .expect("the web service should acquire a port");
        let api = acquire_port_in(state.path(), &scoped_service("multi-service", "apps/api"))
            .expect("the API service should acquire a port");

        assert_ne!(root.port(), web.port());
        assert_ne!(root.port(), api.port());
        assert_ne!(web.port(), api.port());
    }

    #[test]
    fn repository_registration_is_idempotent_and_tracks_a_move() {
        let state = TestDirectory::new();
        let repositories = TestDirectory::new();
        let original = repositories.path().join("original.git");
        let moved = repositories.path().join("moved.git");
        fs::create_dir(&original).expect("original Git directory should be created");
        fs::create_dir(&moved).expect("moved Git directory should be created");
        let repository_id = repository_id("repository");

        register_repository_in(state.path(), &repository_id, &original)
            .expect("repository should be registered");
        register_repository_in(state.path(), &repository_id, &original)
            .expect("repeated registration should be idempotent");
        fs::remove_dir(&original).expect("original Git directory should be removed");
        register_repository_in(state.path(), &repository_id, &moved)
            .expect("a moved repository should refresh its registration");

        let snapshot = snapshot_in(state.path()).expect("runtime state should be readable");
        assert_eq!(snapshot.repositories().len(), 1);
        assert_eq!(
            snapshot.repositories()[0].common_directory(),
            moved.as_path()
        );
    }

    #[test]
    fn one_live_repository_identity_cannot_name_two_directories() {
        let state = TestDirectory::new();
        let repositories = TestDirectory::new();
        let first = repositories.path().join("first.git");
        let second = repositories.path().join("second.git");
        fs::create_dir(&first).expect("first Git directory should be created");
        fs::create_dir(&second).expect("second Git directory should be created");
        let repository_id = repository_id("duplicated");

        register_repository_in(state.path(), &repository_id, &first)
            .expect("first repository should be registered");
        let error = register_repository_in(state.path(), &repository_id, &second)
            .expect_err("a duplicate live identity should be rejected");

        assert!(matches!(error, Error::RepositoryIdentityConflict { .. }));
    }

    #[test]
    fn a_new_identity_replaces_the_stale_identity_for_one_directory() {
        let state = TestDirectory::new();
        let repositories = TestDirectory::new();
        let directory = repositories.path().join("repository.git");
        fs::create_dir(&directory).expect("Git directory should be created");

        register_repository_in(state.path(), &repository_id("old"), &directory)
            .expect("old identity should be registered");
        register_repository_in(state.path(), &repository_id("new"), &directory)
            .expect("new identity should replace the old one");

        let snapshot = snapshot_in(state.path()).expect("runtime state should be readable");
        assert_eq!(snapshot.repositories().len(), 1);
        assert_eq!(snapshot.repositories()[0].repository_id().as_str(), "new");
    }

    #[test]
    fn concurrent_registrations_do_not_lose_an_update() {
        let state = TestDirectory::new();
        let repositories = TestDirectory::new();
        let first = repositories.path().join("first.git");
        let second = repositories.path().join("second.git");
        fs::create_dir(&first).expect("first Git directory should be created");
        fs::create_dir(&second).expect("second Git directory should be created");
        let state_path = std::sync::Arc::new(state.path().to_owned());

        let registrations = [("first", first), ("second", second)]
            .into_iter()
            .map(|(identity, directory)| {
                let state_path = std::sync::Arc::clone(&state_path);
                std::thread::spawn(move || {
                    register_repository_in(&state_path, &repository_id(identity), &directory)
                })
            })
            .collect::<Vec<_>>();
        for registration in registrations {
            registration
                .join()
                .expect("registration thread should not panic")
                .expect("repository should be registered");
        }

        let snapshot = snapshot_in(state.path()).expect("runtime state should be readable");
        assert_eq!(snapshot.repositories().len(), 2);
    }

    #[test]
    fn duplicate_registered_directories_are_rejected_as_corrupt_state() {
        let state = TestDirectory::new();
        let repositories = TestDirectory::new();
        let directory = repositories.path().join("repository.git");
        fs::create_dir(&directory).expect("Git directory should be created");
        fs::write(state.path().join(super::STATE_LOCK_FILE), [])
            .expect("state lock should be created");
        let stored = serde_json::json!({
            "repositories": [
                { "id": "first", "common_directory": directory },
                { "id": "second", "common_directory": directory },
            ],
            "services": [],
        });
        fs::write(
            state.path().join(super::state::FILE_NAME),
            stored.to_string(),
        )
        .expect("invalid state should be written");

        let error = snapshot_in(state.path()).expect_err("invalid state should be rejected");

        assert!(matches!(error, Error::InvalidState { .. }));
    }

    #[test]
    fn persisted_state_requires_its_coordination_lock() {
        let state = TestDirectory::new();
        fs::write(
            state.path().join(super::state::FILE_NAME),
            r#"{"repositories":[],"services":[]}"#,
        )
        .expect("state should be written");

        let error = snapshot_in(state.path()).expect_err("uncoordinated state should be rejected");

        assert!(matches!(error, Error::MissingStateLock { .. }));
    }

    #[test]
    fn persisted_state_names_every_repository_and_service_component() {
        let state = TestDirectory::new();
        let repositories = TestDirectory::new();
        let directory = repositories.path().join("repository.git");
        fs::create_dir(&directory).expect("Git directory should be created");
        let repository_id = repository_id("runtime-test-repository");
        register_repository_in(state.path(), &repository_id, &directory)
            .expect("repository should be registered");
        drop(
            acquire_port_in(state.path(), &scoped_service("worktree", "apps/web"))
                .expect("service should acquire a port"),
        );

        let bytes = fs::read(state.path().join(super::state::FILE_NAME))
            .expect("state file should be readable");
        let stored: serde_json::Value =
            serde_json::from_slice(&bytes).expect("state file should contain JSON");

        assert_eq!(stored["repositories"][0]["id"], "runtime-test-repository");
        assert_eq!(
            stored["repositories"][0]["common_directory"],
            directory.to_string_lossy().as_ref()
        );
        assert_eq!(
            stored["services"][0]["repository_id"],
            "runtime-test-repository"
        );
        assert_eq!(stored["services"][0]["worktree_id"], "worktree");
        assert_eq!(stored["services"][0]["scope"], "apps/web");
        assert_eq!(stored["services"][0]["slot"], 1);
        assert!(stored["services"][0]["port"].is_u64());
        let entries = fs::read_dir(state.path())
            .expect("state directory should be readable")
            .map(|entry| {
                entry
                    .expect("state entry should be readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            entries,
            BTreeSet::from([
                "ports".to_owned(),
                "services".to_owned(),
                "state.json".to_owned(),
                super::STATE_LOCK_FILE.to_owned(),
            ])
        );
    }

    #[test]
    fn unregistering_a_repository_preserves_its_service_assignment() {
        let state = TestDirectory::new();
        let repositories = TestDirectory::new();
        let directory = repositories.path().join("repository.git");
        fs::create_dir(&directory).expect("Git directory should be created");
        let repository_id = repository_id("runtime-test-repository");
        register_repository_in(state.path(), &repository_id, &directory)
            .expect("repository should be registered");
        let lease = acquire_port_in(state.path(), &scoped_service("worktree", "apps/web"))
            .expect("service should acquire a port");
        drop(lease);

        unregister_repository_in(state.path(), Some(&repository_id), &directory)
            .expect("repository should be unregistered");
        unregister_repository_in(state.path(), Some(&repository_id), &directory)
            .expect("repeated unregistration should be idempotent");

        let snapshot = snapshot_in(state.path()).expect("runtime state should be readable");
        assert!(snapshot.repositories().is_empty());
        assert_eq!(snapshot.services().len(), 1);
    }

    #[test]
    fn a_live_directory_can_be_unregistered_without_a_git_identity() {
        let state = TestDirectory::new();
        let repositories = TestDirectory::new();
        let directory = repositories.path().join("repository.git");
        fs::create_dir(&directory).expect("Git directory should be created");
        let repository_id = repository_id("runtime-test-repository");
        register_repository_in(state.path(), &repository_id, &directory)
            .expect("repository should be registered");

        unregister_repository_in(state.path(), None, &directory)
            .expect("the live directory should identify its registration");

        let snapshot = snapshot_in(state.path()).expect("runtime state should be readable");
        assert!(snapshot.repositories().is_empty());
    }

    #[test]
    fn unregistration_does_not_remove_the_same_identity_at_another_live_directory() {
        let state = TestDirectory::new();
        let repositories = TestDirectory::new();
        let registered = repositories.path().join("registered.git");
        let copied = repositories.path().join("copied.git");
        fs::create_dir(&registered).expect("registered Git directory should be created");
        fs::create_dir(&copied).expect("copied Git directory should be created");
        let repository_id = repository_id("runtime-test-repository");
        register_repository_in(state.path(), &repository_id, &registered)
            .expect("repository should be registered");

        unregister_repository_in(state.path(), Some(&repository_id), &copied)
            .expect("unregistration should not affect another live directory");

        let snapshot = snapshot_in(state.path()).expect("runtime state should be readable");
        assert_eq!(snapshot.repositories()[0].common_directory(), registered);
    }

    #[test]
    fn snapshots_report_lock_ownership_without_probing_the_port() {
        let state = TestDirectory::new();
        let lease = acquire_port_in(state.path(), &scoped_service("active", "apps/web"))
            .expect("service should acquire a port");

        let active = snapshot_in(state.path()).expect("active state should be readable");
        assert_eq!(active.services()[0].lease(), LeaseState::Active);
        drop(lease);

        let inactive = snapshot_in(state.path()).expect("inactive state should be readable");
        assert_eq!(inactive.services()[0].lease(), LeaseState::Inactive);
    }

    #[test]
    fn an_absent_state_directory_is_an_empty_read_only_snapshot() {
        let parent = TestDirectory::new();
        let state = parent.path().join("absent");

        let snapshot = snapshot_in(&state).expect("absent state should be empty");

        assert_eq!(snapshot, super::RuntimeSnapshot::default());
        assert!(!state.exists());
    }

    fn root_service(worktree: &str) -> ServiceIdentity {
        ServiceIdentity::new(
            worktree_identity(worktree),
            ServiceScope::from_relative_path(Path::new(""))
                .expect("test service scope should be valid"),
        )
    }

    fn scoped_service(worktree: &str, scope: &str) -> ServiceIdentity {
        scoped_service_in("runtime-test-repository", worktree, scope)
    }

    fn scoped_service_in(repository: &str, worktree: &str, scope: &str) -> ServiceIdentity {
        ServiceIdentity::new(
            WorktreeIdentity::new(
                RepositoryId::new(repository.to_owned())
                    .expect("test repository identity should be valid"),
                worktree.to_owned(),
            )
            .expect("test worktree identity should be valid"),
            ServiceScope::from_relative_path(Path::new(scope))
                .expect("test service scope should be valid"),
        )
    }

    fn worktree_identity(worktree: &str) -> WorktreeIdentity {
        WorktreeIdentity::new(
            RepositoryId::new("runtime-test-repository".to_owned())
                .expect("test repository identity should be valid"),
            worktree.to_owned(),
        )
        .expect("test identity should be valid")
    }

    fn repository_id(value: &str) -> RepositoryId {
        RepositoryId::new(value.to_owned()).expect("test repository identity should be valid")
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..100 {
                let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "ragavan-runtime-test-{}-{sequence}",
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
