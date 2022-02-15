use crate::digest;
use sha2::{Digest, Sha256};
use std::{
    fmt::{self, Display, Formatter},
    io,
    path::PathBuf,
};
use tokio::{fs, io::AsyncReadExt};
use tracing::{debug, info};
use url::Url;

#[derive(Debug)]
pub enum Error {
    /// A downloaded file does not have the expected checksum.
    ChecksumMismatch {
        /// The URL of the downloaded file.
        url: Url,
    },

    Io {
        source: io::Error,
        /// The path that was being acted on when the input/output error occurred.
        path: PathBuf,
    },

    /// A HTTP response contained a non-success status code.
    Http {
        status: reqwest::StatusCode,
        /// The URL that the response was received from.
        url: Url,
    },

    Reqwest(reqwest::Error),
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Self::Reqwest(error)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChecksumMismatch { url } => write!(
                f,
                "downloaded file did not have expected checksum for {}",
                url.as_str()
            ),

            Self::Io { source, path } => {
                source.fmt(f)?;
                write!(f, " for {}", path.to_string_lossy())
            }

            Self::Http { status, url } => {
                write!(f, "a http response had a {} status for {}", status, url)
            }

            Self::Reqwest(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, path: _ } => Some(source),
            Self::Reqwest(error) => error.source(),
            _ => None,
        }
    }
}

/// Specifies how existing download artefacts should be handled.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PreservationStrategy {
    /// Always preserve an existing download.
    Always,
    /// Preserve an existing download when the checksum matches.
    Checksum,
}

// Specifies download options.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Options {
    pub preserve: PreservationStrategy,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            preserve: PreservationStrategy::Always,
        }
    }
}

/// Represents a downloadable artefact.
#[derive(Debug)]
pub struct Download {
    pub url: Url,
    pub destination: PathBuf,
    pub checksum: digest::Sha256,
}

impl Download {
    /// Runs a download.
    pub async fn run(&self, client: &reqwest::Client, options: Options) -> Result<(), Error> {
        match fs::metadata(&self.destination).await {
            Ok(_) => match options.preserve {
                PreservationStrategy::Always => {
                    debug!("skipped integrity checking");
                    info!("already downloaded");
                    return Ok(());
                }

                PreservationStrategy::Checksum => {
                    let mut bytes = Vec::new();
                    let mut file =
                        fs::File::open(&self.destination)
                            .await
                            .map_err(|error| Error::Io {
                                source: error,
                                path: self.destination.clone(),
                            })?;

                    file.read_to_end(&mut bytes)
                        .await
                        .map_err(|error| Error::Io {
                            source: error,
                            path: self.destination.clone(),
                        })?;

                    if Sha256::digest(bytes).as_ref() == self.checksum.0 {
                        info!("already downloaded");
                        return Ok(());
                    }
                }
            },

            Err(error) => {
                if error.kind() != io::ErrorKind::NotFound {
                    return Err(Error::Io {
                        source: error,
                        path: self.destination.clone(),
                    });
                }
            }
        }

        let response = client.get(self.url.clone()).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Http {
                status,
                url: self.url.clone(),
            });
        }

        let bytes = response.bytes().await?;
        if Sha256::digest(&bytes).as_ref() != self.checksum.0 {
            return Err(Error::ChecksumMismatch {
                url: self.url.clone(),
            });
        }

        fs::create_dir_all(
            self.destination
                .parent()
                .expect("destination should have a parent"),
        )
        .await
        .map_err(|error| Error::Io {
            source: error,
            path: self.destination.clone(),
        })?;

        fs::write(&self.destination, bytes)
            .await
            .map_err(|error| Error::Io {
                source: error,
                path: self.destination.clone(),
            })?;

        info!("downloaded");
        Ok(())
    }
}
