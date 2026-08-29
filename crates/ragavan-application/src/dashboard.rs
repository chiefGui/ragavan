use crate::Error;
use crate::{Enrollment, LeaseState, RepositoryId};
use ragavan_git::{RepositoryIdentity, RepositoryInspection, WorktreeInspection};
use ragavan_runtime::{RuntimeSnapshot, ServiceAssignment};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Select which repositories a dashboard describes.
pub enum DashboardScope<'a> {
    /// Describe every repository and service known to Ragavan.
    All,
    /// Describe the repository containing the supplied directory.
    Repository(&'a Path),
}

#[derive(Debug, Eq, PartialEq)]
/// A one-shot management view of Ragavan's repositories and services.
pub struct Dashboard {
    repositories: Vec<DashboardRepository>,
}

impl Dashboard {
    /// Return repositories in deterministic directory and identity order.
    pub fn repositories(&self) -> &[DashboardRepository] {
        &self.repositories
    }
}

#[derive(Debug, Eq, PartialEq)]
/// One repository in a dashboard.
pub struct DashboardRepository {
    id: Option<RepositoryId>,
    observed_id: Option<RepositoryId>,
    registered_directory: Option<PathBuf>,
    common_directory: Option<PathBuf>,
    state: RepositoryState,
    worktrees: Vec<DashboardWorktree>,
}

impl DashboardRepository {
    /// Return the identity Ragavan expects for this repository, when known.
    pub fn id(&self) -> Option<&RepositoryId> {
        self.id.as_ref()
    }

    /// Return a different identity observed in Git, when one conflicts with the expected identity.
    pub fn observed_id(&self) -> Option<&RepositoryId> {
        self.observed_id.as_ref()
    }

    /// Return the other live Git directory registered under the observed identity, when conflicting.
    pub fn registered_directory(&self) -> Option<&Path> {
        self.registered_directory.as_deref()
    }

    /// Return the repository's registered Git common directory, when available.
    pub fn common_directory(&self) -> Option<&Path> {
        self.common_directory.as_deref()
    }

    /// Return the repository's reconciled management state.
    pub fn state(&self) -> RepositoryState {
        self.state
    }

    /// Return worktrees in deterministic path and identity order.
    pub fn worktrees(&self) -> &[DashboardWorktree] {
        &self.worktrees
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// A repository's reconciled management state.
pub enum RepositoryState {
    /// Git and Ragavan agree that the repository is enabled.
    Enabled,
    /// The repository is available but not enabled.
    Disabled,
    /// The registered Git directory cannot be inspected.
    Unavailable,
    /// Git has a missing or malformed identity where Ragavan expects a valid one.
    InvalidIdentity,
    /// The registered or observed repository identities conflict.
    IdentityMismatch,
    /// Service assignments remain for an identity with no registered repository.
    Unregistered,
}

impl From<Enrollment> for RepositoryState {
    fn from(enrollment: Enrollment) -> Self {
        match enrollment {
            Enrollment::Enabled => Self::Enabled,
            Enrollment::Disabled => Self::Disabled,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
/// One Git worktree in a dashboard.
pub struct DashboardWorktree {
    id: Option<String>,
    path: Option<PathBuf>,
    state: WorktreeState,
    services: Vec<DashboardService>,
}

impl DashboardWorktree {
    /// Return the stable worktree identity, when Git can determine it.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// Return the worktree path, when it remains known.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Return the worktree's availability.
    pub fn state(&self) -> WorktreeState {
        self.state
    }

    /// Return services in deterministic scope order.
    pub fn services(&self) -> &[DashboardService] {
        &self.services
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Whether a worktree can currently be inspected.
pub enum WorktreeState {
    /// Git can inspect the worktree.
    Available,
    /// The worktree is missing, prunable, or known only through retained assignments.
    Unavailable,
}

#[derive(Debug, Eq, PartialEq)]
/// One managed development service in a dashboard.
pub struct DashboardService {
    scope: Option<String>,
    port: u16,
    lease: LeaseState,
}

impl DashboardService {
    /// Return the service's repository-relative package scope, or `None` for the root service.
    pub fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }

    /// Return the service's stable assigned port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Return whether Ragavan currently holds the service's coordination lease.
    ///
    /// An active lease does not assert process health or network reachability.
    pub fn lease(&self) -> LeaseState {
        self.lease
    }
}

pub(crate) fn load(scope: DashboardScope<'_>) -> Result<Dashboard, Error> {
    match scope {
        DashboardScope::All => load_all(),
        DashboardScope::Repository(directory) => load_repository(directory),
    }
}

fn load_all() -> Result<Dashboard, Error> {
    let snapshot = ragavan_runtime::snapshot()?;
    let mut services = services_by_repository(snapshot.services());
    let mut repositories = Vec::new();

    for registration in snapshot.repositories() {
        let repository_services = services
            .remove(registration.repository_id())
            .unwrap_or_default();
        repositories.push(registered_repository(
            &snapshot,
            registration.repository_id().clone(),
            registration.common_directory().to_owned(),
            repository_services,
        )?);
    }
    for (repository_id, services) in services {
        repositories.push(unregistered_repository(repository_id, services));
    }

    sort_repositories(&mut repositories);
    Ok(Dashboard { repositories })
}

fn load_repository(directory: &Path) -> Result<Dashboard, Error> {
    let repository = ragavan_git::locate_repository(directory)?
        .ok_or_else(Error::dashboard_repository_required)?;
    let snapshot = ragavan_runtime::snapshot()?;
    let common_directory = repository.common_directory().to_owned();
    let repository = match ragavan_git::inspect_repository(&common_directory)? {
        Some(inspection) => {
            let observed_id = inspection.identity().repository_id();
            let registered_id = snapshot
                .registration_for_directory(&common_directory)?
                .map(|registration| registration.repository_id().clone());
            let registered_directory = observed_id
                .map(|repository_id| {
                    snapshot.conflicting_repository_directory(repository_id, &common_directory)
                })
                .transpose()?
                .flatten()
                .map(PathBuf::from);
            let service_repository_id = registered_id.as_ref().or_else(|| {
                if registered_directory.is_none() {
                    observed_id
                } else {
                    None
                }
            });
            let services = services_for_repository(snapshot.services(), service_repository_id);
            let repository_id = registered_id.or_else(|| observed_id.cloned());
            repository_from_inspection(
                repository_id,
                common_directory,
                services,
                inspection,
                registered_directory,
            )
        }
        None => unavailable_repository(None, common_directory, Vec::new()),
    };

    Ok(Dashboard {
        repositories: vec![repository],
    })
}

fn services_for_repository(
    services: &[ServiceAssignment],
    repository_id: Option<&RepositoryId>,
) -> Vec<ServiceAssignment> {
    repository_id.map_or_else(Vec::new, |repository_id| {
        services
            .iter()
            .filter(|service| service.identity().worktree().repository_id() == repository_id)
            .cloned()
            .collect()
    })
}

fn registered_repository(
    snapshot: &RuntimeSnapshot,
    repository_id: RepositoryId,
    common_directory: PathBuf,
    services: Vec<ServiceAssignment>,
) -> Result<DashboardRepository, Error> {
    Ok(match ragavan_git::inspect_repository(&common_directory)? {
        Some(inspection) => {
            let registered_directory = inspection
                .identity()
                .repository_id()
                .map(|observed_id| {
                    snapshot.conflicting_repository_directory(observed_id, &common_directory)
                })
                .transpose()?
                .flatten()
                .map(PathBuf::from);
            repository_from_inspection(
                Some(repository_id),
                common_directory,
                services,
                inspection,
                registered_directory,
            )
        }
        None => unavailable_repository(Some(repository_id), common_directory, services),
    })
}

fn repository_from_inspection(
    expected_id: Option<RepositoryId>,
    common_directory: PathBuf,
    services: Vec<ServiceAssignment>,
    inspection: RepositoryInspection,
    registered_directory: Option<PathBuf>,
) -> DashboardRepository {
    let (identity_state, observed_id) = repository_state(
        expected_id.as_ref(),
        inspection.identity(),
        inspection.enrollment(),
    );
    let state = if registered_directory.is_some() {
        RepositoryState::IdentityMismatch
    } else {
        identity_state
    };
    let worktrees = inspected_worktrees(&inspection, services);

    DashboardRepository {
        id: expected_id.or_else(|| inspection.identity().repository_id().cloned()),
        observed_id,
        registered_directory,
        common_directory: Some(common_directory),
        state,
        worktrees,
    }
}

fn repository_state(
    expected_id: Option<&RepositoryId>,
    observed: &RepositoryIdentity,
    enrollment: Enrollment,
) -> (RepositoryState, Option<RepositoryId>) {
    match (expected_id, observed) {
        (Some(expected), RepositoryIdentity::Valid(observed)) if expected == observed => {
            (enrollment.into(), None)
        }
        (Some(_), RepositoryIdentity::Valid(observed)) => {
            (RepositoryState::IdentityMismatch, Some(observed.clone()))
        }
        (None, RepositoryIdentity::Valid(_)) => (enrollment.into(), None),
        (None, RepositoryIdentity::Missing) if enrollment == Enrollment::Disabled => {
            (RepositoryState::Disabled, None)
        }
        (_, RepositoryIdentity::Missing | RepositoryIdentity::Invalid) => {
            (RepositoryState::InvalidIdentity, None)
        }
    }
}

fn inspected_worktrees(
    inspection: &RepositoryInspection,
    services: Vec<ServiceAssignment>,
) -> Vec<DashboardWorktree> {
    let mut services = services_by_worktree(services);
    let mut worktrees = Vec::new();

    for worktree in inspection.worktrees() {
        match worktree {
            WorktreeInspection::Available { id, path } => {
                let worktree_services = services.remove(id).unwrap_or_default();
                worktrees.push(DashboardWorktree {
                    id: Some(id.clone()),
                    path: Some(path.clone()),
                    state: WorktreeState::Available,
                    services: dashboard_services(worktree_services),
                });
            }
            WorktreeInspection::Unavailable { path } => {
                worktrees.push(DashboardWorktree {
                    id: None,
                    path: Some(path.clone()),
                    state: WorktreeState::Unavailable,
                    services: Vec::new(),
                });
            }
        }
    }
    for (worktree_id, services) in services {
        worktrees.push(DashboardWorktree {
            id: Some(worktree_id),
            path: None,
            state: WorktreeState::Unavailable,
            services: dashboard_services(services),
        });
    }
    sort_worktrees(&mut worktrees);
    worktrees
}

fn unavailable_repository(
    repository_id: Option<RepositoryId>,
    common_directory: PathBuf,
    services: Vec<ServiceAssignment>,
) -> DashboardRepository {
    DashboardRepository {
        id: repository_id,
        observed_id: None,
        registered_directory: None,
        common_directory: Some(common_directory),
        state: RepositoryState::Unavailable,
        worktrees: assignment_only_worktrees(services),
    }
}

fn unregistered_repository(
    repository_id: RepositoryId,
    services: Vec<ServiceAssignment>,
) -> DashboardRepository {
    DashboardRepository {
        id: Some(repository_id),
        observed_id: None,
        registered_directory: None,
        common_directory: None,
        state: RepositoryState::Unregistered,
        worktrees: assignment_only_worktrees(services),
    }
}

fn assignment_only_worktrees(services: Vec<ServiceAssignment>) -> Vec<DashboardWorktree> {
    let mut worktrees = services_by_worktree(services)
        .into_iter()
        .map(|(worktree_id, services)| DashboardWorktree {
            id: Some(worktree_id),
            path: None,
            state: WorktreeState::Unavailable,
            services: dashboard_services(services),
        })
        .collect::<Vec<_>>();
    sort_worktrees(&mut worktrees);
    worktrees
}

fn services_by_repository(
    services: &[ServiceAssignment],
) -> BTreeMap<RepositoryId, Vec<ServiceAssignment>> {
    let mut grouped = BTreeMap::<_, Vec<_>>::new();
    for service in services {
        grouped
            .entry(service.identity().worktree().repository_id().clone())
            .or_default()
            .push(service.clone());
    }
    grouped
}

fn services_by_worktree(
    services: Vec<ServiceAssignment>,
) -> BTreeMap<String, Vec<ServiceAssignment>> {
    let mut grouped = BTreeMap::<_, Vec<_>>::new();
    for service in services {
        grouped
            .entry(service.identity().worktree().worktree_id().to_owned())
            .or_default()
            .push(service);
    }
    grouped
}

fn dashboard_services(services: Vec<ServiceAssignment>) -> Vec<DashboardService> {
    let mut services = services
        .into_iter()
        .map(|service| DashboardService {
            scope: service
                .identity()
                .scope()
                .relative_path()
                .map(str::to_owned),
            port: service.port().get(),
            lease: service.lease(),
        })
        .collect::<Vec<_>>();
    services.sort_by(|left, right| left.scope.cmp(&right.scope));
    services
}

fn sort_repositories(repositories: &mut [DashboardRepository]) {
    repositories.sort_by(|left, right| {
        left.common_directory
            .is_none()
            .cmp(&right.common_directory.is_none())
            .then_with(|| left.common_directory.cmp(&right.common_directory))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn sort_worktrees(worktrees: &mut [DashboardWorktree]) {
    worktrees.sort_by(|left, right| {
        left.path
            .is_none()
            .cmp(&right.path.is_none())
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.id.cmp(&right.id))
    });
}
