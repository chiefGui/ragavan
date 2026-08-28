#![forbid(unsafe_code)]

use ragavan_diagnostics::{Detail, Diagnostic};
use std::{
    fmt,
    path::{Component, Path},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Enrollment {
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Whether Ragavan currently holds a service's coordination lease.
pub enum LeaseState {
    /// The service lock is currently held.
    Active,
    /// The stable assignment remains, but no process holds its service lock.
    Inactive,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RepositoryId(String);

impl RepositoryId {
    pub fn new(value: String) -> Result<Self, IdentityError> {
        if value.is_empty() {
            return Err(IdentityError::EmptyRepository);
        }

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepositoryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WorktreeIdentity {
    repository_id: RepositoryId,
    worktree_id: String,
}

impl WorktreeIdentity {
    pub fn new(repository_id: RepositoryId, worktree_id: String) -> Result<Self, IdentityError> {
        if worktree_id.is_empty() {
            return Err(IdentityError::EmptyWorktree);
        }

        Ok(Self {
            repository_id,
            worktree_id,
        })
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn worktree_id(&self) -> &str {
        &self.worktree_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    EmptyRepository,
    EmptyWorktree,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRepository => formatter.write_str("repository identity cannot be empty"),
            Self::EmptyWorktree => formatter.write_str("worktree identity cannot be empty"),
        }
    }
}

impl std::error::Error for IdentityError {}

impl Diagnostic for IdentityError {
    fn code(&self) -> &'static str {
        match self {
            Self::EmptyRepository => "identity.repository.empty",
            Self::EmptyWorktree => "identity.worktree.empty",
        }
    }

    fn details(&self) -> Vec<Detail> {
        vec![Detail::text(
            "identity",
            match self {
                Self::EmptyRepository => "repository",
                Self::EmptyWorktree => "worktree",
            },
        )]
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ServiceScope(Option<String>);

impl ServiceScope {
    pub fn from_relative_path(path: &Path) -> Result<Self, ServiceScopeError> {
        let mut normalized = String::new();

        for component in path.components() {
            let component = match component {
                Component::CurDir => continue,
                Component::Normal(component) => component,
                Component::ParentDir => return Err(ServiceScopeError::ParentTraversal),
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ServiceScopeError::NonRelativePath);
                }
            };
            let component = component
                .to_str()
                .ok_or(ServiceScopeError::NonUnicodePath)?;
            if !normalized.is_empty() {
                normalized.push('/');
            }
            normalized.push_str(component);
        }

        Ok(Self((!normalized.is_empty()).then_some(normalized)))
    }

    pub fn relative_path(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceScopeError {
    NonRelativePath,
    ParentTraversal,
    NonUnicodePath,
}

impl fmt::Display for ServiceScopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonRelativePath => formatter.write_str("service scope must be a relative path"),
            Self::ParentTraversal => {
                formatter.write_str("service scope cannot traverse to a parent directory")
            }
            Self::NonUnicodePath => {
                formatter.write_str("service scope path must contain valid Unicode")
            }
        }
    }
}

impl std::error::Error for ServiceScopeError {}

impl Diagnostic for ServiceScopeError {
    fn code(&self) -> &'static str {
        match self {
            Self::NonRelativePath => "service_scope.non_relative",
            Self::ParentTraversal => "service_scope.parent_traversal",
            Self::NonUnicodePath => "service_scope.non_unicode",
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ServiceIdentity {
    worktree: WorktreeIdentity,
    scope: ServiceScope,
}

impl ServiceIdentity {
    pub fn new(worktree: WorktreeIdentity, scope: ServiceScope) -> Self {
        Self { worktree, scope }
    }

    pub fn worktree(&self) -> &WorktreeIdentity {
        &self.worktree
    }

    pub fn scope(&self) -> &ServiceScope {
        &self.scope
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Port(u16);

impl Port {
    pub fn new(value: u16) -> Option<Self> {
        (value != 0).then_some(Self(value))
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

impl fmt::Display for Port {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchPlan {
    additional_arguments: Vec<String>,
}

impl LaunchPlan {
    pub fn with_additional_arguments(additional_arguments: Vec<String>) -> Self {
        Self {
            additional_arguments,
        }
    }

    pub fn into_additional_arguments(self) -> Vec<String> {
        self.additional_arguments
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IdentityError, RepositoryId, ServiceIdentity, ServiceScope, ServiceScopeError,
        WorktreeIdentity,
    };
    use std::path::Path;

    #[test]
    fn a_service_is_rooted_or_scoped_within_one_worktree() {
        let worktree = worktree();
        let root = ServiceIdentity::new(
            worktree.clone(),
            ServiceScope::from_relative_path(Path::new(""))
                .expect("the root service scope should be valid"),
        );
        let scoped = ServiceIdentity::new(
            worktree.clone(),
            ServiceScope::from_relative_path(Path::new("apps/web"))
                .expect("the nested service scope should be valid"),
        );

        assert_eq!(root.worktree(), &worktree);
        assert_eq!(root.scope().relative_path(), None);
        assert_eq!(scoped.worktree(), &worktree);
        assert_eq!(scoped.scope().relative_path(), Some("apps/web"));
    }

    #[test]
    fn service_scopes_normalize_relative_paths() {
        let scope = ServiceScope::from_relative_path(Path::new("./apps/./web"))
            .expect("the relative service path should be valid");

        assert_eq!(scope.relative_path(), Some("apps/web"));
    }

    #[test]
    fn service_scopes_reject_paths_outside_the_worktree() {
        let parent = ServiceScope::from_relative_path(Path::new("../web"))
            .expect_err("parent traversal should be rejected");
        let absolute = ServiceScope::from_relative_path(Path::new("/apps/web"))
            .expect_err("an absolute path should be rejected");

        assert_eq!(parent, ServiceScopeError::ParentTraversal);
        assert_eq!(absolute, ServiceScopeError::NonRelativePath);
    }

    #[test]
    fn repository_identifiers_cannot_be_empty() {
        assert_eq!(
            RepositoryId::new(String::new()),
            Err(IdentityError::EmptyRepository)
        );
        assert_eq!(
            RepositoryId::new("repository".to_owned())
                .expect("the repository identity should be valid")
                .as_str(),
            "repository"
        );
    }

    fn worktree() -> WorktreeIdentity {
        WorktreeIdentity::new(
            RepositoryId::new("repository".to_owned())
                .expect("the repository identity should be valid"),
            "worktree".to_owned(),
        )
        .expect("the worktree identity should be valid")
    }
}
