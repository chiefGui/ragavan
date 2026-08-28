#![forbid(unsafe_code)]

mod assignments;

use self::assignments::PortAssignments;
use ragavan_core::{Port, ServiceIdentity};
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
const ALLOCATOR_LOCK_FILE: &str = "allocator.lock";
const SERVICE_LOCKS_DIRECTORY: &str = "services";
const PORT_LOCKS_DIRECTORY: &str = "ports";

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

fn acquire_port_in(state_directory: &Path, identity: &ServiceIdentity) -> Result<PortLease, Error> {
    let service_locks = state_directory.join(SERVICE_LOCKS_DIRECTORY);
    let port_locks = state_directory.join(PORT_LOCKS_DIRECTORY);
    for directory in [state_directory, &service_locks, &port_locks] {
        fs::create_dir_all(directory).map_err(|source| Error::CreateState {
            path: directory.to_owned(),
            source,
        })?;
    }

    let allocator_lock_path = state_directory.join(ALLOCATOR_LOCK_FILE);
    let allocator_lock = open_lock(&allocator_lock_path)?;
    allocator_lock
        .lock()
        .map_err(|source| Error::LockAllocator {
            path: allocator_lock_path,
            source,
        })?;

    let mut assignments = PortAssignments::read(state_directory)?;
    let assignment = assignments.assignment(identity)?;

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
    assignments.remember(identity, assignment, port.get())?;

    drop(allocator_lock);
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
    let repository_slot = stable_hash(worktree.repository_id()) % range_size;
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
    CreateState {
        path: PathBuf,
        source: io::Error,
    },
    OpenLock {
        path: PathBuf,
        source: io::Error,
    },
    LockAllocator {
        path: PathBuf,
        source: io::Error,
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
    ReadAssignments {
        path: PathBuf,
        source: io::Error,
    },
    ParseAssignments {
        path: PathBuf,
        source: serde_json::Error,
    },
    SerializeAssignments {
        path: PathBuf,
        source: serde_json::Error,
    },
    InvalidAssignments {
        path: PathBuf,
        detail: String,
    },
    WriteAssignments {
        path: PathBuf,
        source: io::Error,
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
            Self::LockAllocator { path, source } => write!(
                formatter,
                "could not lock Ragavan's port allocator at {}: {source}",
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
            Self::ReadAssignments { path, source } => write!(
                formatter,
                "could not read Ragavan port assignments from {}: {source}",
                path.display()
            ),
            Self::ParseAssignments { path, source } => write!(
                formatter,
                "could not parse Ragavan port assignments at {}: {source}",
                path.display()
            ),
            Self::SerializeAssignments { path, source } => write!(
                formatter,
                "could not serialize Ragavan port assignments for {}: {source}",
                path.display()
            ),
            Self::InvalidAssignments { path, detail } => write!(
                formatter,
                "could not use Ragavan port assignments at {}: {detail}",
                path.display()
            ),
            Self::WriteAssignments { path, source } => write!(
                formatter,
                "could not write Ragavan port assignments to {}: {source}",
                path.display()
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
            | Self::OpenLock { source, .. }
            | Self::LockAllocator { source, .. }
            | Self::LockService { source, .. }
            | Self::LockPort { source, .. }
            | Self::ReadAssignments { source, .. }
            | Self::WriteAssignments { source, .. }
            | Self::CheckPort { source, .. }
            | Self::RunProcess { source, .. } => Some(source),
            Self::ParseAssignments { source, .. } | Self::SerializeAssignments { source, .. } => {
                Some(source)
            }
            Self::StateHomeUnavailable { .. }
            | Self::InvalidStateHome { .. }
            | Self::ServiceAlreadyRunning
            | Self::InvalidAssignments { .. }
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
            Self::CreateState { .. } => "runtime.state.create",
            Self::OpenLock { .. } => "runtime.lock.open",
            Self::LockAllocator { .. } => "runtime.lock.allocator",
            Self::LockService { .. } => "runtime.lock.service",
            Self::LockPort { .. } => "runtime.lock.port",
            Self::ServiceAlreadyRunning => "runtime.service.already_running",
            Self::ReadAssignments { .. } => "runtime.assignments.read",
            Self::ParseAssignments { .. } => "runtime.assignments.parse",
            Self::SerializeAssignments { .. } => "runtime.assignments.serialize",
            Self::InvalidAssignments { .. } => "runtime.assignments.invalid",
            Self::WriteAssignments { .. } => "runtime.assignments.write",
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
            | Self::OpenLock { path, .. }
            | Self::LockAllocator { path, .. }
            | Self::LockService { path, .. }
            | Self::LockPort { path, .. }
            | Self::ReadAssignments { path, .. }
            | Self::ParseAssignments { path, .. }
            | Self::SerializeAssignments { path, .. }
            | Self::WriteAssignments { path, .. } => {
                vec![Detail::text("path", path.display().to_string())]
            }
            Self::InvalidAssignments { path, detail } => vec![
                Detail::text("path", path.display().to_string()),
                Detail::text("reason", detail),
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
    use super::{Error, acquire_port_in};
    use ragavan_core::{ServiceIdentity, ServiceScope, WorktreeIdentity};
    use std::{
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
            WorktreeIdentity::new(repository.to_owned(), worktree.to_owned())
                .expect("test worktree identity should be valid"),
            ServiceScope::from_relative_path(Path::new(scope))
                .expect("test service scope should be valid"),
        )
    }

    fn worktree_identity(worktree: &str) -> WorktreeIdentity {
        WorktreeIdentity::new("runtime-test-repository".to_owned(), worktree.to_owned())
            .expect("test identity should be valid")
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
