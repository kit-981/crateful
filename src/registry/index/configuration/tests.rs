use super::*;
use crate::digest::Sha256;

#[test]
fn test_deserialise_configuration() {
    let data = r#"{
  "dl": "https://static.crates.io/api/v1/crates",
  "api": "https://crates.io"
}"#;

    let expected = Configuration {
        template: "https://static.crates.io/api/v1/crates".into(),
    };

    let output =
        Configuration::from_slice(data.as_bytes()).expect("failed to deserialise configuration");

    assert_eq!(output, expected);
}

#[test]
fn test_deserialise_corrupt_configuration_with_missing_fields() {
    let data = r#""#;
    assert!(Configuration::from_slice(data.as_bytes()).is_err());
}

#[test]
fn test_get_default_crate_url() {
    let crate_ = Crate {
        name: String::from("example"),
        version: String::from("1.0.0"),
        checksum: Sha256(
            hex::decode("fae02128713e38ea8d4973b9d8944273dbd6db36cee7e1bc0e41ee5022933783")
                .expect("failed to decode hex string")
                .try_into()
                .expect("hex string has invalid length"),
        ),
    };

    let configuration = Configuration {
        template: "https://static.crates.io/api/v1/crates".into(),
    };

    let expected = Url::parse("https://static.crates.io/api/v1/crates/example/1.0.0/download")
        .expect("failed to parse url");

    assert_eq!(
        configuration
            .locate(&crate_)
            .expect("failed to locate crate"),
        expected
    );
}

#[test]
fn test_get_templated_crate_url() {
    let crate_ = Crate {
        name: String::from("EXAMPLE"),
        version: String::from("1.0.0"),
        checksum: Sha256(
            hex::decode("fae02128713e38ea8d4973b9d8944273dbd6db36cee7e1bc0e41ee5022933783")
                .expect("failed to decode hex string")
                .try_into()
                .expect("hex string has invalid length"),
        ),
    };

    let configuration = Configuration {
        template: "https://static.crates.io/api/v1/crates/{crate}/{version}/{prefix}/{lowerprefix}/{sha256-checksum}".into(),
    };

    let expected =
        Url::parse("https://static.crates.io/api/v1/crates/EXAMPLE/1.0.0/EX/AM/ex/am/fae02128713e38ea8d4973b9d8944273dbd6db36cee7e1bc0e41ee5022933783")
            .expect("failed to parse url");

    assert_eq!(
        configuration
            .locate(&crate_)
            .expect("failed to locate crate"),
        expected
    );
}
