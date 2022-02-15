use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Hash)]
pub struct Sha256(#[serde(with = "hex")] pub [u8; 32]);
