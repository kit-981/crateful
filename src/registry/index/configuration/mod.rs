#[cfg(test)]
pub mod tests;

use crate::registry::index::package::Crate;
use serde::Deserialize;
use std::{
    convert::Into,
    error::Error,
    fmt::{self, Display, Formatter},
};
use url::Url;

#[derive(Debug)]
pub struct DeserialiseConfigurationError {
    inner: serde_json::Error,
}

impl From<serde_json::Error> for DeserialiseConfigurationError {
    fn from(error: serde_json::Error) -> Self {
        Self { inner: error }
    }
}

impl Display for DeserialiseConfigurationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl Error for DeserialiseConfigurationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.inner.source()
    }
}

#[derive(Debug)]
pub struct TemplateUrlError {
    source: url::ParseError,
    crate_: Crate,
}

impl Display for TemplateUrlError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to generate valid URL for crate with name {}, version {}, and checksum {}",
            self.crate_.name,
            self.crate_.version,
            hex::encode(self.crate_.checksum.0)
        )
    }
}

impl Error for TemplateUrlError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// A registry index configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Hash)]
pub struct Configuration {
    #[serde(rename(deserialize = "dl"))]
    pub template: String,
}

impl Configuration {
    /// Returns the remote location of `crate_`.
    pub fn locate(&self, crate_: &Crate) -> Result<Url, TemplateUrlError> {
        let prefix = crate_.prefix();
        let templated = self
            .template
            .as_str()
            .replace("{crate}", &crate_.name)
            .replace("{version}", &crate_.version)
            .replace("{prefix}", &prefix)
            .replace("{lowerprefix}", &prefix.to_lowercase())
            .replace("{sha256-checksum}", &hex::encode(&crate_.checksum.0));

        let string = if templated == self.template {
            // The documentation mentions that if none of the markers are present then
            // /{crate}/{version}/download is appended to the configuration download url.
            let mut default = self.template.clone();
            default.push_str(&format!(
                "/{}/{}/{}",
                &crate_.name, &crate_.version, "download"
            ));
            default
        } else {
            templated
        };

        // TODO: It would be ideal to guarantee that this is successful by validating the
        // configuration template and crates when they are each deserialised.
        Url::parse(&string).map_err(|error| TemplateUrlError {
            source: error,
            crate_: crate_.clone(),
        })
    }

    /// Deserialises a configuration from a slice.
    pub fn from_slice(slice: &[u8]) -> Result<Self, DeserialiseConfigurationError> {
        serde_json::from_slice(slice).map_err(Into::into)
    }
}
