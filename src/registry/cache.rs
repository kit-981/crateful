use crate::{
    download::{self, Download},
    registry::index::{
        self,
        configuration::{Configuration, TemplateUrlError},
        package::{Crate, Package},
        ChangeKind, Index,
    },
};
use futures::{stream, StreamExt, TryStreamExt};
use reqwest::Client;
use std::{
    error::Error,
    fmt::{self, Display, Formatter},
    io,
    num::NonZeroUsize,
    path::{Path, PathBuf},
};
use tokio::fs;
use tracing::{debug, info_span, warn};
use tracing_futures::Instrument;
use url::Url;

/// The error type for pruning directories.
#[derive(Debug)]
#[non_exhaustive]
pub enum PruneDirectoriesError {
    Io(io::Error),
    /// It is not possible to traverse from the start directory to the finish directory.
    TraversalIsImpossible,
}

impl From<io::Error> for PruneDirectoriesError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl Display for PruneDirectoriesError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::TraversalIsImpossible => write!(
                f,
                "impossible to traverse from the start directory to the finish directory"
            ),
        }
    }
}

impl Error for PruneDirectoriesError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => error.source(),
            _ => None,
        }
    }
}

/// Traverses upwards from `from` to `until` and removes any empty directories found directly on
/// this traversal. `until` is never removed.
async fn prune_directories(mut from: &Path, until: &Path) -> Result<(), PruneDirectoriesError> {
    if !from.starts_with(until) {
        return Err(PruneDirectoriesError::TraversalIsImpossible);
    }

    while from != until {
        debug_assert!(from.starts_with(until));

        // Check if the directory is empty.
        if fs::read_dir(from).await?.next_entry().await?.is_none() {
            fs::remove_dir(from).await?;
        }

        // Traverse upwards.
        from = from.parent().expect("a parent must exist");
    }

    Ok(())
}

#[derive(Debug)]
pub struct CrateDownloadError {
    source: download::Error,
    name: String,
    version: String,
}

impl Display for CrateDownloadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to download crate {} version {}",
            self.name, self.version
        )
    }
}

impl Error for CrateDownloadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum RefreshCacheError {
    CrateDownload(CrateDownloadError),
    GetConfiguration(index::GetConfigurationError),
    GetPackages(index::GetPackagesError),
    MalformedDownloadTemplate(TemplateUrlError),
}

impl From<CrateDownloadError> for RefreshCacheError {
    fn from(error: CrateDownloadError) -> Self {
        Self::CrateDownload(error)
    }
}

impl From<TemplateUrlError> for RefreshCacheError {
    fn from(error: TemplateUrlError) -> Self {
        Self::MalformedDownloadTemplate(error)
    }
}

impl From<index::GetConfigurationError> for RefreshCacheError {
    fn from(error: index::GetConfigurationError) -> Self {
        Self::GetConfiguration(error)
    }
}

impl From<index::GetPackagesError> for RefreshCacheError {
    fn from(error: index::GetPackagesError) -> Self {
        Self::GetPackages(error)
    }
}

impl Display for RefreshCacheError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedDownloadTemplate(_) => {
                write!(f, "configuration download template is malformed")
            }
            Self::CrateDownload(error) => error.fmt(f),
            Self::GetConfiguration(error) => error.fmt(f),
            Self::GetPackages(error) => error.fmt(f),
        }
    }
}

impl Error for RefreshCacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MalformedDownloadTemplate(error) => Some(error),
            Self::CrateDownload(error) => error.source(),
            Self::GetConfiguration(error) => error.source(),
            Self::GetPackages(error) => error.source(),
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum UpdateError {
    CommitUpdate(index::CommitUpdateError),
    CrateDownload(CrateDownloadError),
    GetConfiguration(index::GetConfigurationError),
    GetUpdate(index::GetUpdateError),
    Io(io::Error),
    MalformedDownloadTemplate(TemplateUrlError),
    PruneDirectories(PruneDirectoriesError),
}

impl From<index::GetUpdateError> for UpdateError {
    fn from(error: index::GetUpdateError) -> Self {
        Self::GetUpdate(error)
    }
}

impl From<index::GetConfigurationError> for UpdateError {
    fn from(error: index::GetConfigurationError) -> Self {
        Self::GetConfiguration(error)
    }
}

impl From<TemplateUrlError> for UpdateError {
    fn from(error: TemplateUrlError) -> Self {
        Self::MalformedDownloadTemplate(error)
    }
}

impl From<CrateDownloadError> for UpdateError {
    fn from(error: CrateDownloadError) -> Self {
        Self::CrateDownload(error)
    }
}

impl From<index::CommitUpdateError> for UpdateError {
    fn from(error: index::CommitUpdateError) -> Self {
        Self::CommitUpdate(error)
    }
}

impl From<io::Error> for UpdateError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<PruneDirectoriesError> for UpdateError {
    fn from(error: PruneDirectoriesError) -> Self {
        Self::PruneDirectories(error)
    }
}

impl Display for UpdateError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommitUpdate(error) => error.fmt(f),
            Self::CrateDownload(error) => error.fmt(f),
            Self::GetConfiguration(error) => error.fmt(f),
            Self::GetUpdate(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
            Self::MalformedDownloadTemplate(_) => {
                write!(f, "configuration download template is malformed")
            }
            Self::PruneDirectories(error) => error.fmt(f),
        }
    }
}

impl Error for UpdateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MalformedDownloadTemplate(error) => Some(error),
            Self::CommitUpdate(error) => error.source(),
            Self::CrateDownload(error) => error.source(),
            Self::GetConfiguration(error) => error.source(),
            Self::GetUpdate(error) => error.source(),
            Self::Io(error) => error.source(),
            Self::PruneDirectories(error) => error.source(),
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum CreateCacheError {
    CloneIndex(index::CloneIndexError),
}

impl Display for CreateCacheError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CloneIndex(error) => error.fmt(f),
        }
    }
}

impl Error for CreateCacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CloneIndex(error) => error.source(),
        }
    }
}

impl From<index::CloneIndexError> for CreateCacheError {
    fn from(error: index::CloneIndexError) -> Self {
        Self::CloneIndex(error)
    }
}

#[derive(Debug)]
pub struct LoadCacheError(index::OpenIndexError);

impl Display for LoadCacheError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "failed to load cache")
    }
}

impl Error for LoadCacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.source()
    }
}

impl From<index::OpenIndexError> for LoadCacheError {
    fn from(error: index::OpenIndexError) -> Self {
        Self(error)
    }
}

#[derive(Debug)]
pub struct Cache {
    path: PathBuf,
    index: Index,
}

impl Cache {
    /// The directory in the cache that holds the index.
    pub const INDEX_SUBDIRECTORY: &'static str = "index";

    /// The directory in the cache that holds the crates.
    pub const CRATES_SUBDIRECTORY: &'static str = "crates";

    /// Returns the path to the crates directory.
    #[must_use]
    pub fn crates_path(&self) -> PathBuf {
        self.path.join(Self::CRATES_SUBDIRECTORY)
    }

    /// Creates a new cache.
    pub async fn new(path: PathBuf, index: Url) -> Result<Self, CreateCacheError> {
        let index = Index::from_url(index, path.join(Self::INDEX_SUBDIRECTORY)).await?;
        Ok(Self { path, index })
    }

    /// Returns a cache from a file system path.
    pub async fn from_path(path: PathBuf) -> Result<Self, LoadCacheError> {
        let index = Index::from_path(path.join(Self::INDEX_SUBDIRECTORY)).await?;
        Ok(Self { path, index })
    }

    /// Locates a crate in the cache. The crate is not guaranteed to exist.
    #[must_use]
    pub fn locate_crate(&self, item: &Crate) -> PathBuf {
        self.crates_path()
            .join(item.name.as_str())
            .join(item.version.as_str())
            .join("download")
    }

    /// Creates a download for a crate.
    fn download(
        &self,
        configuration: &Configuration,
        item: &Crate,
    ) -> Result<Download, TemplateUrlError> {
        let url = configuration.locate(item)?;
        let destination = self.locate_crate(item);

        Ok(Download {
            url,
            destination,
            checksum: item.checksum,
        })
    }

    /// Refreshes the cache.
    ///
    /// The packages that should be in the cache are enumerated and (re)downloaded.
    pub async fn refresh(
        &self,
        client: &Client,
        options: download::Options,
        jobs: NonZeroUsize,
    ) -> Result<(), RefreshCacheError> {
        let configuration = &self.index.configuration().await?;

        stream::iter(
            self.index
                .packages()
                .await?
                .into_iter()
                .flat_map(Package::into_crates)
                .map(Ok),
        )
        .try_for_each_concurrent(jobs.get(), |each| {
            let name = each.name.clone();
            let version = each.version.clone();

            async move {
                if let Err(error) = self
                    .download(configuration, &each)?
                    .run(client, options)
                    .await
                {
                    match &error {
                        // There are crates in the crates.io index and registry with inconsistent
                        // checksums.
                        download::Error::ChecksumMismatch { url: _ }
                        // There are known issues with crates.io where it will respond with
                        // unsuccessful HTTP statuses (eg. 403) for crates that are listed in the
                        // index.
                        | download::Error::Http { status: _, url: _ } => {
                            warn!("{}", error);
                        }

                        _ => {
                            return Err(CrateDownloadError {
                                source: error,
                                name: each.name.clone(),
                                version: each.version.clone(),
                            }
                            .into())
                        }
                    }
                }

                Ok::<_, RefreshCacheError>(())
            }
            .instrument(info_span!(
                "download",
                name = name.as_str(),
                version = version.as_str()
            ))
        })
        .await
    }

    /// Updates the cache.
    ///
    /// # Errors
    ///
    /// Pending changes that are reported by the [`Index`] are acted on by downloading or removing
    /// files and the [`Index`] is only updated when these operations have completed
    /// successfully. This ensures that intermittent network or file system failures do not leave
    /// the cache in a permanently inconsistent state.
    ///
    /// The state of the cache may be temporarily inconsistent when an update fails. This can
    /// generally be rectified by updating again until the operation is successful.
    ///
    /// ## Index Corruption
    ///
    /// It is possible that the cache may become permanently inconsistent if the index becomes
    /// corrupt in any new commit since the cache was initialised. Index corruption makes it
    /// impossible to deduce what crates were added, removed, or changed. Currently, this can only
    /// be rectified by creating a new cache.
    pub async fn update(
        &self,
        client: &Client,
        options: download::Options,
        jobs: NonZeroUsize,
    ) -> Result<(), UpdateError> {
        let pending = self.index.update().await?;

        // It's possible that an update will modify the configuration.
        //
        // It is difficult to recover from a configuration being aggressively deprecated and
        // disabled as `Self::refresh` must always be run before updates are fetched to ensure that
        // the cache is consistent. If the current configuration is disabled then `Self::refresh`
        // will fail.
        //
        // This may be resolved in the future by enumerating updates before refreshing the cache and
        // using the latest available configuration when refreshing the cache and applying an
        // update.
        let configuration = &self.index.configuration().await?;

        stream::iter(pending.changes())
            .map(Ok)
            .try_for_each_concurrent(jobs.get(), |change| {
                async move {
                    match change.kind {
                        ChangeKind::Added => {
                            if let Err(error) = self
                                .download(configuration, &change.on)?
                                .run(client, options)
                                .await
                            {
                                match &error {
                                    download::Error::ChecksumMismatch { url: _ }
                                    | download::Error::Http { status: _, url: _ } => {
                                        warn!("{}", error);
                                    }

                                    _ => {
                                        return Err(CrateDownloadError {
                                            source: error,
                                            name: change.on.name.clone(),
                                            version: change.on.version.clone(),
                                        }
                                        .into())
                                    }
                                }
                            }

                            debug!("processed an addition");
                        }

                        ChangeKind::Removed => {
                            let location = self.locate_crate(&change.on);

                            // Remove the artefact and any obsoleted directories if they exist. It's
                            // possible that this change was already operated on but not committed
                            // to the index.
                            match fs::metadata(&location).await {
                                Ok(_) => fs::remove_file(&location).await?,
                                Err(error) => {
                                    if error.kind() != io::ErrorKind::NotFound {
                                        return Err(error.into());
                                    }
                                }
                            }

                            prune_directories(
                                location.parent().expect("file path must have a parent"),
                                &self.path,
                            )
                            .await?;

                            debug!("processed a removal");
                        }

                        ChangeKind::Modified => {
                            // Remove the artefact. It's possible that this change was already
                            // operated on but not committed to the index.
                            let location = self.locate_crate(&change.on);
                            match fs::metadata(&location).await {
                                Ok(_) => fs::remove_file(&location).await?,
                                Err(error) => {
                                    if error.kind() != io::ErrorKind::NotFound {
                                        return Err(error.into());
                                    }
                                }
                            }

                            if let Err(error) = self
                                .download(configuration, &change.on)?
                                .run(client, options)
                                .await
                            {
                                match &error {
                                    download::Error::ChecksumMismatch { url: _ }
                                    | download::Error::Http { status: _, url: _ } => {
                                        warn!("{}", error);
                                    }

                                    _ => {
                                        return Err(CrateDownloadError {
                                            source: error,
                                            name: change.on.name.clone(),
                                            version: change.on.version.clone(),
                                        }
                                        .into())
                                    }
                                }
                            }

                            debug!("processed a modification");
                        }
                    };

                    Ok::<_, UpdateError>(())
                }
                .instrument(info_span!(
                    "change",
                    name = change.on.name.as_str(),
                    version = change.on.version.as_str()
                ))
            })
            .await?;

        pending.commit().await?;
        debug!("committed an update to the index");

        Ok(())
    }
}
