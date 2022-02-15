use super::*;

#[test]
fn test_deserialise_package_with_single_crate() {
    let data = r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"bae3d8de1b7fd1fef6c2da3130a7d06d32499fd5292a9c1309681ac79e98c643","features":{},"yanked":false}"#;
    let expected = Package({
        let mut set = AHashSet::new();
        set.insert(Crate {
            name: String::from("a"),
            version: String::from("0.0.1"),
            checksum: Sha256(
                hex::decode("bae3d8de1b7fd1fef6c2da3130a7d06d32499fd5292a9c1309681ac79e98c643")
                    .expect("failed to decode hex string")
                    .try_into()
                    .expect("hex string has invalid length"),
            ),
        });

        set
    });

    let output = Package::from_slice(data.as_bytes()).expect("failed to deserialise package");
    assert_eq!(output, expected);
}

#[test]
fn test_deserialise_package_with_single_crate_with_trailing_newline() {
    let data = r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"bae3d8de1b7fd1fef6c2da3130a7d06d32499fd5292a9c1309681ac79e98c643","features":{},"yanked":false}
"#;
    let expected = Package({
        let mut set = AHashSet::new();
        set.insert(Crate {
            name: String::from("a"),
            version: String::from("0.0.1"),
            checksum: Sha256(
                hex::decode("bae3d8de1b7fd1fef6c2da3130a7d06d32499fd5292a9c1309681ac79e98c643")
                    .expect("failed to decode hex string")
                    .try_into()
                    .expect("hex string has invalid length"),
            ),
        });

        set
    });

    let output = Package::from_slice(data.as_bytes()).expect("failed to deserialise package");
    assert_eq!(output, expected);
}

#[test]
fn test_deserialise_package_with_multiple_crates() {
    let data = r#"{"name":"b","vers":"0.1.0","deps":[],"cksum":"fae02128713e38ea8d4973b9d8944273dbd6db36cee7e1bc0e41ee5022933783","features":{},"yanked":false}
{"name":"b","vers":"0.2.0","deps":[],"cksum":"ad71822f94ff0251011da9d7c63248c2520e6a69e56d457be0679b4fe81cbada","features":{},"yanked":false,"links":null}"#;
    let expected = Package({
        let mut set = AHashSet::new();
        set.insert(Crate {
            name: String::from("b"),
            version: String::from("0.1.0"),
            checksum: Sha256(
                hex::decode("fae02128713e38ea8d4973b9d8944273dbd6db36cee7e1bc0e41ee5022933783")
                    .expect("failed to decode hex string")
                    .try_into()
                    .expect("hex string has invalid length"),
            ),
        });
        set.insert(Crate {
            name: String::from("b"),
            version: String::from("0.2.0"),
            checksum: Sha256(
                hex::decode("ad71822f94ff0251011da9d7c63248c2520e6a69e56d457be0679b4fe81cbada")
                    .expect("failed to decode hex string")
                    .try_into()
                    .expect("hex string has invalid length"),
            ),
        });

        set
    });

    let output = Package::from_slice(data.as_bytes()).expect("failed to deserialise package");
    assert_eq!(output, expected);
}

#[test]
fn test_deserialise_corrupt_package_with_missing_fields() {
    assert!(Package::from_slice(b"{}").is_err());
}

#[test]
fn test_get_single_crate_prefix() {
    let crate_ = Crate {
        name: String::from("a"),
        version: String::from("1.0.0"),
        checksum: Sha256(
            hex::decode("fae02128713e38ea8d4973b9d8944273dbd6db36cee7e1bc0e41ee5022933783")
                .expect("failed to decode hex string")
                .try_into()
                .expect("hex string has invalid length"),
        ),
    };

    assert_eq!(crate_.prefix().as_str(), "1");
}

#[test]
fn test_get_double_crate_prefix() {
    let crate_ = Crate {
        name: String::from("bb"),
        version: String::from("1.0.0"),
        checksum: Sha256(
            hex::decode("fae02128713e38ea8d4973b9d8944273dbd6db36cee7e1bc0e41ee5022933783")
                .expect("failed to decode hex string")
                .try_into()
                .expect("hex string has invalid length"),
        ),
    };

    assert_eq!(crate_.prefix().as_str(), "2");
}

#[test]
fn test_get_triple_crate_prefix() {
    let crate_ = Crate {
        name: String::from("ccc"),
        version: String::from("1.0.0"),
        checksum: Sha256(
            hex::decode("fae02128713e38ea8d4973b9d8944273dbd6db36cee7e1bc0e41ee5022933783")
                .expect("failed to decode hex string")
                .try_into()
                .expect("hex string has invalid length"),
        ),
    };

    assert_eq!(crate_.prefix().as_str(), "3/c");
}

#[test]
fn test_get_quad_crate_prefix() {
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

    assert_eq!(crate_.prefix().as_str(), "ex/am");
}
