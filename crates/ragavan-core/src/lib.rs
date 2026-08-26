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
