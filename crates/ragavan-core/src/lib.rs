#![forbid(unsafe_code)]

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Enrollment {
    Enabled,
    Disabled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeIdentity {
    repository_id: String,
    worktree_id: String,
}

impl WorktreeIdentity {
    pub fn new(repository_id: String, worktree_id: String) -> Result<Self, IdentityError> {
        if repository_id.is_empty() {
            return Err(IdentityError::EmptyRepository);
        }
        if worktree_id.is_empty() {
            return Err(IdentityError::EmptyWorktree);
        }

        Ok(Self {
            repository_id,
            worktree_id,
        })
    }

    pub fn repository_id(&self) -> &str {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceIdentity {
    worktree: WorktreeIdentity,
    scope: Option<String>,
}

impl ServiceIdentity {
    pub fn root(worktree: WorktreeIdentity) -> Self {
        Self {
            worktree,
            scope: None,
        }
    }

    pub fn scoped(worktree: WorktreeIdentity, scope: String) -> Result<Self, ServiceIdentityError> {
        if scope.is_empty() {
            return Err(ServiceIdentityError::EmptyScope);
        }

        Ok(Self {
            worktree,
            scope: Some(scope),
        })
    }

    pub fn worktree(&self) -> &WorktreeIdentity {
        &self.worktree
    }

    pub fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceIdentityError {
    EmptyScope,
}

impl fmt::Display for ServiceIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyScope => formatter.write_str("service scope cannot be empty"),
        }
    }
}

impl std::error::Error for ServiceIdentityError {}

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
    use super::{ServiceIdentity, ServiceIdentityError, WorktreeIdentity};

    #[test]
    fn a_service_is_rooted_or_scoped_within_one_worktree() {
        let worktree = worktree();
        let root = ServiceIdentity::root(worktree.clone());
        let scoped = ServiceIdentity::scoped(worktree.clone(), "apps/web".to_owned())
            .expect("the service scope should be valid");

        assert_eq!(root.worktree(), &worktree);
        assert_eq!(root.scope(), None);
        assert_eq!(scoped.worktree(), &worktree);
        assert_eq!(scoped.scope(), Some("apps/web"));
    }

    #[test]
    fn a_scoped_service_requires_a_nonempty_scope() {
        let error = ServiceIdentity::scoped(worktree(), String::new())
            .expect_err("an empty service scope should be rejected");

        assert_eq!(error, ServiceIdentityError::EmptyScope);
    }

    fn worktree() -> WorktreeIdentity {
        WorktreeIdentity::new("repository".to_owned(), "worktree".to_owned())
            .expect("the worktree identity should be valid")
    }
}
