#[cfg(test)]
pub mod tests;

use crate::digest::Sha256;
use ahash::AHashSet;
use serde::Deserialize;
use std::{
    convert::Into,
    error::Error,
    fmt::{self, Display, Formatter},
    str::{self, Utf8Error},
};

/// A crate is uniquely identified by its name, version, and hash. A crate key identifies a crate
/// only by its name and version.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct CrateKey {
    /// The name of the crate.
    pub name: String,
    /// The version of the crate.
    pub version: String,
}

#[derive(Debug)]
pub struct DeserialiseCrateError {
    inner: serde_json::Error,
}

impl From<serde_json::Error> for DeserialiseCrateError {
    fn from(error: serde_json::Error) -> Self {
        Self { inner: error }
    }
}

impl Display for DeserialiseCrateError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl Error for DeserialiseCrateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.inner.source()
    }
}

/// A crate is a minimum required subset of the registry metadata describing a crate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Hash)]
pub struct Crate {
    /// The name of the crate.
    pub name: String,
    /// The version of the crate.
    #[serde(rename = "vers")]
    pub version: String,
    /// The checksum of the crate.
    #[serde(rename = "cksum")]
    pub checksum: Sha256,
}

impl Crate {
    /// Returns the URL prefix for the crate.
    #[must_use]
    pub fn prefix(&self) -> String {
        let chars: Vec<_> = self.name.chars().take(4).collect();
        match chars.len() {
            1 => String::from("1"),
            2 => String::from("2"),
            3 => format!("3/{}", chars[0]),
            4 => format!(
                "{}/{}",
                chars[0..2].iter().collect::<String>(),
                chars[2..4].iter().collect::<String>()
            ),
            _ => unreachable!("unexpected length"),
        }
    }

    /// Returns the crate as a crate key.
    #[must_use]
    pub fn key(&self) -> CrateKey {
        CrateKey {
            name: self.name.clone(),
            version: self.version.clone(),
        }
    }

    /// Deserialises a crate from a string slice.
    pub fn from_str(str: &str) -> Result<Self, DeserialiseCrateError> {
        serde_json::from_str(str).map_err(Into::into)
    }
}

#[derive(Debug)]
pub enum DeserialisePackageError {
    Json {
        source: DeserialiseCrateError,
        line: usize,
    },

    Utf8(Utf8Error),
}

impl Display for DeserialisePackageError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json { source: _, line } => {
                write!(f, "invalid json for line {}", line)
            }

            Self::Utf8(error) => error.fmt(f),
        }
    }
}

impl Error for DeserialisePackageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json { source, line: _ } => Some(source),
            Self::Utf8(error) => error.source(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct Package(AHashSet<Crate>);

impl Package {
    /// Returns the crates.
    pub fn into_crates(self) -> impl Iterator<Item = Crate> {
        self.0.into_iter()
    }

    /// Deserialises a package from a string slice.
    pub fn from_str(str: &str) -> Result<Self, DeserialisePackageError> {
        let crates = str
            .lines()
            .enumerate()
            .map(|(line, slice)| {
                Crate::from_str(slice.trim()).map_err(|error| DeserialisePackageError::Json {
                    source: error,
                    line,
                })
            })
            .collect::<Result<AHashSet<_>, _>>()?;

        Ok(Self(crates))
    }

    /// Deserialises a package from a slice of bytes.
    pub fn from_slice(slice: &[u8]) -> Result<Self, DeserialisePackageError> {
        Self::from_str(std::str::from_utf8(slice).map_err(DeserialisePackageError::Utf8)?)
    }
}
