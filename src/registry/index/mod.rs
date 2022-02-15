pub mod configuration;
pub mod package;

use ahash::AHashMap;
use configuration::{Configuration, DeserialiseConfigurationError};
use git2::{Branch, Delta, DiffDelta, FetchOptions, Oid, Repository};
use itertools::Itertools;
use package::{Crate, CrateKey, Package};
use std::{
    convert::Into,
    error::Error,
    fmt::{self, Debug, Display, Formatter},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::task;
use tracing::debug;
use url::Url;

#[derive(Debug)]
#[non_exhaustive]
pub enum OpenIndexError {
    Git(git2::Error),
}

impl From<git2::Error> for OpenIndexError {
    fn from(error: git2::Error) -> Self {
        Self::Git(error)
    }
}

impl Display for OpenIndexError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(error) => Display::fmt(error, f),
        }
    }
}

impl Error for OpenIndexError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Git(error) => error.source(),
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum CloneIndexError {
    Git(git2::Error),
}

impl From<git2::Error> for CloneIndexError {
    fn from(error: git2::Error) -> Self {
        Self::Git(error)
    }
}

impl Display for CloneIndexError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(error) => Display::fmt(error, f),
        }
    }
}

impl Error for CloneIndexError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Git(error) => error.source(),
        }
    }
}

/// A package is corrupt.
#[derive(Debug)]
pub struct CorruptPackageError {
    source: package::DeserialisePackageError,
    path: PathBuf,
}

impl Display for CorruptPackageError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "corrupt package at {}", self.path.to_string_lossy())
    }
}

impl Error for CorruptPackageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum GetPackagesError {
    Git(git2::Error),
    CorruptPackage(CorruptPackageError),
}

impl From<git2::Error> for GetPackagesError {
    fn from(error: git2::Error) -> Self {
        Self::Git(error)
    }
}

impl From<CorruptPackageError> for GetPackagesError {
    fn from(error: CorruptPackageError) -> Self {
        Self::CorruptPackage(error)
    }
}

impl Display for GetPackagesError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(error) => Display::fmt(error, f),
            Self::CorruptPackage(error) => Display::fmt(error, f),
        }
    }
}

impl Error for GetPackagesError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Git(error) => error.source(),
            Self::CorruptPackage(error) => error.source(),
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum GetUpdateError {
    CorruptPackage(CorruptPackageError),
    Git(git2::Error),
    /// Implementation limitations prevent the index from being interacted with if it uses an
    /// encoding other than UTF-8.
    IndexUsesUnsupportedEncoding,
    UnexpectedIndexState,
}

impl From<git2::Error> for GetUpdateError {
    fn from(error: git2::Error) -> Self {
        Self::Git(error)
    }
}

impl From<CorruptPackageError> for GetUpdateError {
    fn from(error: CorruptPackageError) -> Self {
        Self::CorruptPackage(error)
    }
}

impl Display for GetUpdateError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CorruptPackage(error) => Display::fmt(error, f),
            Self::Git(error) => Display::fmt(error, f),
            Self::IndexUsesUnsupportedEncoding => write!(f, "index uses unsupported encoding"),
            Self::UnexpectedIndexState => write!(f, "unexpected index state"),
        }
    }
}

impl Error for GetUpdateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CorruptPackage(error) => error.source(),
            Self::Git(error) => error.source(),
            Self::UnexpectedIndexState | Self::IndexUsesUnsupportedEncoding => None,
        }
    }
}

/// Describes how a crate in the index was changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ChangeKind {
    /// A crate was added.
    Added,
    /// A crate was removed.
    Removed,
    /// A crate was modified.
    Modified,
}

/// Describes a change to the index. Changes are safe to act on in parallel.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Change {
    /// The crate that was changed.
    pub on: Crate,
    /// The kind of change that happened.
    pub kind: ChangeKind,
}

/// Generates changes from a series of deltas for individual package files.
///
/// # Async
///
/// This is a blocking function and must not be used from an asynchronous context.
#[allow(clippy::too_many_lines)]
fn changes_from_package_trees<'a>(
    repository: &'a Repository,
    deltas: impl Iterator<Item = DiffDelta<'a>> + 'a,
) -> impl Iterator<Item = Result<Change, GetUpdateError>> + 'a {
    deltas
        // At the time of writing, Rust does not support try blocks and this makes it inconvenient
        // to filter elements while propagating errors. This must done separately.
        .filter(|diff| {
            matches!(
                diff.status(),
                Delta::Added | Delta::Deleted | Delta::Modified
            )
        })
        .map(|diff| {
            let (f, s, t) = match diff.status() {
                Delta::Added => (
                    Some(
                        Package::from_slice(repository.find_blob(diff.new_file().id())?.content())
                            .map_err(|error| CorruptPackageError {
                                source: error,
                                path: diff
                                    .new_file()
                                    .path()
                                    .expect("new file path missing")
                                    .to_path_buf(),
                            })?
                            .into_crates()
                            .map(|on| Change {
                                on,
                                kind: ChangeKind::Added,
                            }),
                    ),
                    None,
                    None,
                ),

                Delta::Deleted => (
                    None,
                    Some(
                        Package::from_slice(repository.find_blob(diff.old_file().id())?.content())
                            .map_err(|error| CorruptPackageError {
                                source: error,
                                path: diff
                                    .old_file()
                                    .path()
                                    .expect("old path missing")
                                    .to_path_buf(),
                            })?
                            .into_crates()
                            .map(|on| Change {
                                on,
                                kind: ChangeKind::Removed,
                            }),
                    ),
                    None,
                ),

                Delta::Modified => {
                    // If a package was modified then a crate could be added, removed, or
                    // changed. The old crates are enumerated and compared with the new crates to
                    // determine what change occurred.
                    let mut after =
                        Package::from_slice(repository.find_blob(diff.new_file().id())?.content())
                            .map_err(|error| CorruptPackageError {
                                source: error,
                                path: diff
                                    .new_file()
                                    .path()
                                    .expect("new file path missing")
                                    .to_path_buf(),
                            })?
                            .into_crates()
                            .map(|each| (each.key(), each))
                            .collect::<AHashMap<CrateKey, Crate>>();

                    let mut changes = Vec::new();
                    for before in
                        Package::from_slice(repository.find_blob(diff.old_file().id())?.content())
                            .map_err(|error| CorruptPackageError {
                                source: error,
                                path: diff
                                    .old_file()
                                    .path()
                                    .expect("old file path missing")
                                    .to_path_buf(),
                            })?
                            .into_crates()
                    {
                        let key = before.key();
                        if let Some(after) = after.remove(&key) {
                            // If the key is present in both collections then either the crate was
                            // not changed or the file was modified.
                            if before.checksum != after.checksum {
                                changes.push(Change {
                                    on: after,
                                    kind: ChangeKind::Modified,
                                });
                            }
                        } else {
                            changes.push(Change {
                                on: before,
                                kind: ChangeKind::Removed,
                            });
                        }
                    }

                    // All remaining crates in `after` were added.
                    changes.reserve(after.len());
                    changes.extend(after.into_iter().map(|(_, on)| Change {
                        on,
                        kind: ChangeKind::Added,
                    }));

                    (None, None, Some(changes.into_iter()))
                }

                _ => unreachable!(),
            };

            // This allows the function to "return" any of the iterators without collecting them or
            // using dynamic dispatch.
            Ok(f.into_iter()
                .flatten()
                .chain(s.into_iter().flatten().chain(t.into_iter().flatten())))
        })
        .flatten_ok()
}

#[derive(Debug)]
#[non_exhaustive]
pub enum CommitUpdateError {
    Git(git2::Error),
}

impl From<git2::Error> for CommitUpdateError {
    fn from(error: git2::Error) -> Self {
        Self::Git(error)
    }
}

impl Display for CommitUpdateError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(error) => Display::fmt(error, f),
        }
    }
}

impl Error for CommitUpdateError {}

/// represents a pending update to the index.
pub struct PendingUpdate {
    repository: Arc<Mutex<Repository>>,
    /// The target is the object that HEAD should point to if the update is committed.
    target: Oid,
    changes: Vec<Change>,
}

impl PendingUpdate {
    /// Returns the changes in the pending update.
    pub fn changes(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter()
    }

    /// Commits the update.
    pub async fn commit(self) -> Result<(), CommitUpdateError> {
        task::spawn_blocking(move || {
            let repo = self.repository.lock().expect("lock is poisoned");
            repo.head()?
                .set_target(self.target, "fast forward branch")?;

            debug!("committed update to the index repository");
            Ok(())
        })
        .await
        .expect("panicked while committing update")
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum GetConfigurationError {
    /// The configuration is corrupt.
    Corrupt(DeserialiseConfigurationError),
    Git(git2::Error),
    /// The configuration could not be found.
    NotFound,
}

impl From<DeserialiseConfigurationError> for GetConfigurationError {
    fn from(error: DeserialiseConfigurationError) -> Self {
        Self::Corrupt(error)
    }
}

impl From<git2::Error> for GetConfigurationError {
    fn from(error: git2::Error) -> Self {
        Self::Git(error)
    }
}

impl Display for GetConfigurationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Corrupt(_) => write!(f, "configuration is corrupt"),
            Self::Git(error) => Display::fmt(error, f),
            Self::NotFound => write!(f, "configuration not found"),
        }
    }
}

impl Error for GetConfigurationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Corrupt(error) => Some(error),
            Self::Git(_) | Self::NotFound => None,
        }
    }
}

/// An index is a Git repository containing metadata for a crate registry.
#[derive(Clone)]
pub struct Index {
    repository: Arc<Mutex<Repository>>,
}

impl Index {
    pub const CONFIGURATION_FILENAME: &'static str = "config.json";

    /// Open a registry index from a path.
    pub async fn from_path(path: PathBuf) -> Result<Self, OpenIndexError> {
        task::spawn_blocking(move || Repository::open(path))
            .await
            .expect("panicked while opening the repository")
            .map(|repository| Self {
                repository: Arc::new(Mutex::new(repository)),
            })
            .map_err(Into::into)
    }

    /// Open a registry index from a url. The registry index is cloned to `destination`.
    pub async fn from_url(url: Url, destination: PathBuf) -> Result<Self, CloneIndexError> {
        task::spawn_blocking(move || Repository::clone(url.as_str(), destination))
            .await
            .expect("panicked while cloning the repository")
            .map(|repository| Self {
                repository: Arc::new(Mutex::new(repository)),
            })
            .map_err(Into::into)
    }

    /// Returns the configuration for the index.
    pub async fn configuration(&self) -> Result<Configuration, GetConfigurationError> {
        let repo = self.repository.clone();
        task::spawn_blocking(move || {
            let repo = repo.lock().expect("lock is poisoned");
            let blob = repo.find_blob(
                repo.head()?
                    .peel_to_tree()?
                    .get_name(Self::CONFIGURATION_FILENAME)
                    .ok_or(GetConfigurationError::NotFound)?
                    .id(),
            )?;

            Configuration::from_slice(blob.content()).map_err(Into::into)
        })
        .await
        .expect("panicked while getting the configuration")
    }

    /// Returns a list of packages that are currently held by the index.
    pub async fn packages(&self) -> Result<Vec<Package>, GetPackagesError> {
        let repo = self.repository.clone();
        task::spawn_blocking(move || {
            let repo = repo.lock().expect("lock is poisoned");
            let tree = repo.head()?.peel_to_tree()?;

            tree.iter()
                .filter_map(|entry| {
                    if let Some(name) = entry.name() {
                        // Ignore hidden files.
                        if name.starts_with('.') {
                            return None;
                        }
                    }

                    entry.to_object(&repo).ok()
                })
                // Filter all files in the root directory that are not directories. This ensures
                // that the configuration is not included.
                .filter_map(|obj| obj.into_tree().ok())
                .map(|tree| {
                    repo.diff_tree_to_tree(None, Some(&tree), None)
                        .map_err(GetPackagesError::from)
                })
                .map_ok(|diff| {
                    diff.deltas()
                        .into_iter()
                        .map(|delta| {
                            let file = delta.new_file();
                            let blob = repo.find_blob(file.id())?;
                            Ok::<Package, GetPackagesError>(
                                Package::from_slice(blob.content()).map_err(|error| {
                                    CorruptPackageError {
                                        source: error,
                                        path: file.path().expect("file missing path").to_path_buf(),
                                    }
                                })?,
                            )
                        })
                        .collect::<Vec<_>>()
                        .into_iter()
                })
                .flatten_ok()
                // Result::flatten is experimental.
                .map(|result| match result {
                    Ok(result) => result,
                    Err(error) => Err(error),
                })
                .collect()
        })
        .await
        .expect("panicked while getting the packages")
    }

    /// Stages an update.
    ///
    /// Changes to the index repository are synchronised locally each time an update is staged but
    /// these changes are not applied. [`PendingUpdate`] can be used to enumerate the pending
    /// changes. The update can be committed once the changes have been handled.
    pub async fn update(&self) -> Result<PendingUpdate, GetUpdateError> {
        let locked_repo = self.repository.clone();
        task::spawn_blocking(move || {
            let unlocked_repo = locked_repo.clone();
            let repo = unlocked_repo.lock().expect("lock is poisoned");

            let head = repo.head()?;
            if !head.is_branch() {
                return Err(GetUpdateError::UnexpectedIndexState);
            }

            let name = head
                .name()
                .ok_or(GetUpdateError::IndexUsesUnsupportedEncoding)?;
            let mut remote = repo.find_remote(
                repo.branch_upstream_remote(name)?
                    .as_str()
                    .ok_or(GetUpdateError::IndexUsesUnsupportedEncoding)?,
            )?;

            remote.fetch(&[name], Some(&mut FetchOptions::new()), None)?;
            debug!("fetched the latest changes from the index remote");

            let branch = Branch::wrap(head);
            let upstream = branch.upstream()?;

            let exclude = repo
                .workdir()
                .ok_or(GetUpdateError::UnexpectedIndexState)?
                .join(Self::CONFIGURATION_FILENAME);

            let changes = changes_from_package_trees(
                &repo,
                repo.diff_tree_to_tree(
                    Some(&branch.get().peel_to_tree()?),
                    Some(&upstream.get().peel_to_tree()?),
                    None,
                )?
                .deltas()
                .filter(|delta| {
                    let path = match delta.old_file().path() {
                        Some(path) => Some(path),
                        None => delta.new_file().path(),
                    };

                    path.map_or(true, |path| path != exclude)
                }),
            )
            .collect::<Result<Vec<_>, GetUpdateError>>()?;

            Ok(PendingUpdate {
                target: upstream
                    .get()
                    .target()
                    .ok_or(GetUpdateError::UnexpectedIndexState)?,
                repository: locked_repo,
                changes,
            })
        })
        .await
        .expect("panicked while collecting update")
    }
}

impl Debug for Index {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Index").finish()
    }
}
