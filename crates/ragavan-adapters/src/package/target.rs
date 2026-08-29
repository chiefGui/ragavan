use std::ffi::OsStr;

#[derive(Clone, Copy)]
pub(crate) enum PackageTarget<'a> {
    WorkingDirectory,
    Selected(PackageSelector<'a>),
    MissingValue(&'static str),
    Multiple,
    NonExact(&'a OsStr),
}

impl<'a> PackageTarget<'a> {
    pub(crate) fn select(&mut self, selector: PackageSelector<'a>, option: &'static str) {
        if selector.value().is_empty() {
            *self = Self::MissingValue(option);
            return;
        }

        *self = match *self {
            Self::WorkingDirectory => Self::Selected(selector),
            Self::Selected(existing) if existing == selector => *self,
            Self::MissingValue(_) => *self,
            Self::Selected(_) | Self::Multiple | Self::NonExact(_) => Self::Multiple,
        };
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum PackageSelector<'a> {
    Name(&'a OsStr),
    Directory {
        value: &'a OsStr,
        relative_to: SelectorBase,
    },
    NameOrDirectory {
        value: &'a OsStr,
        relative_to: SelectorBase,
    },
}

impl<'a> PackageSelector<'a> {
    pub(super) fn value(self) -> &'a OsStr {
        match self {
            Self::Name(value)
            | Self::Directory { value, .. }
            | Self::NameOrDirectory { value, .. } => value,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum SelectorBase {
    WorkingDirectory,
    WorktreeRoot,
}
