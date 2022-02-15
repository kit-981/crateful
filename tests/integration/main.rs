#![warn(clippy::all, clippy::cargo, clippy::nursery, clippy::pedantic)]

use core::convert::Into;
use futures::{
    stream::{self, FuturesUnordered},
    StreamExt,
};
use git2::{Index, IndexEntry, IndexTime, Repository, Signature};
use serde::Serialize;
use std::{
    convert::AsRef,
    env, io,
    ops::Range,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::{Arc, Mutex},
};
use tempfile::TempDir;
use tokio::{fs, process::Command, task::spawn_blocking};
use tokio_util::sync::CancellationToken;
use url::Url;
use warp::Filter;

async fn assert_exists(
    paths: impl Iterator<Item = impl AsRef<Path> + Send + Sync> + Send,
    should_exist: bool,
) {
    let mut futures: FuturesUnordered<_> = paths
        .map(|path| async move {
            match fs::metadata(&path).await {
                Ok(_) => assert!(should_exist, "{} exists", path.as_ref().to_string_lossy()),

                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    assert!(
                        !should_exist,
                        "{} does not exist",
                        path.as_ref().to_string_lossy()
                    );
                }

                _ => panic!(
                    "failed to check if {} exists",
                    path.as_ref().to_string_lossy()
                ),
            };
        })
        .collect();

    while futures.next().await.is_some() {}
}

/// A registry index format.
#[derive(Clone, Debug, Serialize, Eq, PartialEq, Hash)]
pub struct IndexFormat {
    #[serde(rename(serialize = "dl"))]
    pub download: String,
}

struct Crateful {
    location: PathBuf,
}

impl Crateful {
    /// Invokes crateful to create a new cache.
    async fn create(&self, path: impl AsRef<Path> + Send + Sync, url: &Url) -> ExitStatus {
        Command::new(&self.location)
            .arg("--path")
            .arg(path.as_ref())
            .arg("new")
            .arg("--url")
            .arg(url.as_str())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .unwrap_or_else(|_| panic!("failed to run {}", self.location.to_string_lossy()))
    }

    /// Invokes crateful to synchronise a cache.
    async fn sync(&self, path: impl AsRef<Path> + Send + Sync) -> ExitStatus {
        Command::new(&self.location)
            .arg("--path")
            .arg(path.as_ref())
            .arg("sync")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .unwrap_or_else(|_| panic!("failed to run {}", self.location.to_string_lossy()))
    }

    /// Invokes crateful to verify a cache.
    async fn verify(&self, path: impl AsRef<Path> + Send + Sync) -> ExitStatus {
        Command::new(&self.location)
            .arg("--path")
            .arg(path.as_ref())
            .arg("verify")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .unwrap_or_else(|_| panic!("failed to run {}", self.location.to_string_lossy()))
    }
}

/// A private collection of test resources.
struct Resources {
    exe: Crateful,
    workspace: TempDir,
}

impl Resources {
    /// Returns a new set of resources.
    #[inline]
    #[must_use]
    fn new() -> Self {
        Self {
            exe: Crateful {
                location: PathBuf::from(env!("CARGO_BIN_EXE_crateful")),
            },
            workspace: TempDir::new().expect("can't get temporary directory"),
        }
    }

    /// Returns the executable.
    #[inline]
    #[must_use]
    pub const fn exe(&self) -> &Crateful {
        &self.exe
    }

    /// Returns the workspace. The workspace is a private temporary directory for storing test
    /// artefacts.
    #[inline]
    #[must_use]
    pub fn workspace(&self) -> &Path {
        self.workspace.path()
    }
}

/// Range of permitted ports for a web server.
const PERMITTED_PORTS: Range<u16> = 1024..2048;

/// A simple abstraction around a Git repository for staging and eventually committing files.
struct Stager<'a> {
    repository: &'a Repository,
    index: Index,
}

impl<'a> Stager<'a> {
    /// Returns a new stager for the repository.
    fn new(repository: &'a Repository) -> Self {
        let index = repository.index().expect("failed to get the index");
        Self { repository, index }
    }

    /// Stages a file.
    fn add(&mut self, path: Vec<u8>, contents: &[u8]) -> &mut Self {
        self.index
            .add(&IndexEntry {
                ctime: IndexTime::new(0, 0),
                mtime: IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode: 0o100_644,
                uid: 0,
                gid: 0,
                file_size: 0,
                id: {
                    self.repository
                        .blob(contents)
                        .expect("failed to write contents to object database")
                },
                flags: 0,
                flags_extended: 0,
                path,
            })
            .expect("failed to stage contents");

        self
    }

    /// Removes a file.
    fn remove(&mut self, path: &Path) -> &mut Self {
        self.index.remove_path(path).expect("failed to remove path");
        self
    }

    /// Commits any staged files.
    fn commit(&mut self) {
        let parent = match self.repository.head() {
            Ok(reference) => Some({
                reference
                    .peel_to_commit()
                    .expect("failed to get commit for HEAD")
            }),
            Err(_) => None,
        };

        let parents = parent.as_ref().into_iter().collect::<Vec<_>>();
        let signature = Signature::now("crateful", "crateful").expect("failed to create signature");

        self.repository
            .commit(
                Some("refs/heads/master"),
                &signature,
                &signature,
                "commit",
                {
                    &self
                        .repository
                        .find_tree(self.index.write_tree().expect("failed to write index"))
                        .expect("failed to write tree")
                },
                parents.as_slice(),
            )
            .expect("failed to commit");

        // The index must be written to the disk.
        self.index.write().expect("failed to write index");
    }
}

#[tokio::test]
async fn test_new() {
    let resources = Resources::new();
    let registry_index = resources.workspace().join("index");
    spawn_blocking({
        let registry_index = registry_index.clone();
        move || {
            let repo =
                Repository::init(&registry_index).expect("failed to initialise registry index");

            Stager::new(&repo)
                .add(b"config.json".to_vec(), {
                    let configuration = IndexFormat {
                        // The download template will never be used.
                        download: "http://127.0.0.1:80".into(),
                    };

                    serde_json::to_vec(&configuration)
                        .expect("failed to serialise index format")
                        .as_slice()
                })
                .commit();
        }
    })
    .await
    .expect("failed to prepare registry index");

    let cache = resources.workspace().join("cache");
    let status = resources
        .exe()
        .create(
            &cache,
            &Url::from_file_path(registry_index).expect("failed to get url for registry index"),
        )
        .await;

    assert!(status.success(), "failed to create cache");
    assert_exists([&cache, &cache.join("index")].into_iter(), true).await;
    assert_exists([cache.join("crates")].into_iter(), false).await;
}

#[tokio::test]
async fn test_sync() {
    let resources = Resources::new();

    let filter = warp::path!(String / String / "download").and_then(
        |name: String, version: String| async move {
            match (name.as_str(), version.as_str()) {
                ("a", "0.0.1") => Ok("0"),
                _ => Err(warp::reject::not_found()),
            }
        },
    );

    let parent = CancellationToken::new();
    let child = &parent.child_token();

    let stream = stream::iter(PERMITTED_PORTS).filter_map(|port| async move {
        let address = ([127, 0, 0, 1], port);
        let token = child.clone();

        match warp::serve(filter)
            .try_bind_with_graceful_shutdown(address, async move { token.cancelled().await })
        {
            Ok((socket, server)) => Some((socket, server)),
            Err(_) => None,
        }
    });

    tokio::pin!(stream);
    let (socket, server) = stream
        .next()
        .await
        .expect("no available port in permitted range");

    let _guard = parent.drop_guard();
    tokio::spawn(server);

    let registry_index = resources.workspace().join("index");
    spawn_blocking({
        let registry_index = registry_index.clone();
        move || {
            let repo =
                Repository::init(&registry_index).expect("failed to initialise registry index");

            Stager::new(&repo)
                .add(b"config.json".to_vec(), {
                    let configuration = IndexFormat {
                        download: format!("http://127.0.0.1:{}", socket.port()),
                    };

                    serde_json::to_vec(&configuration)
                        .expect("failed to serialise index format")
                        .as_slice()
                })
                .add(
                    b"1/a".to_vec(),
                    r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to prepare registry index");

    let cache = resources.workspace().join("cache");
    let status = resources
        .exe()
        .create(
            &cache,
            &Url::from_file_path(registry_index).expect("failed to get url for registry index"),
        )
        .await;

    assert!(status.success(), "failed to create cache");
    assert_exists([&cache, &cache.join("index")].into_iter(), true).await;
    assert_exists([cache.join("crates")].into_iter(), false).await;

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;
}

#[tokio::test]
async fn test_sync_twice() {
    let resources = Resources::new();

    let filter = warp::path!(String / String / "download").and_then(
        |name: String, version: String| async move {
            match (name.as_str(), version.as_str()) {
                ("a", "0.0.1") => Ok("0"),
                _ => Err(warp::reject::not_found()),
            }
        },
    );

    let parent = CancellationToken::new();
    let child = &parent.child_token();

    let stream = stream::iter(PERMITTED_PORTS).filter_map(|port| async move {
        let address = ([127, 0, 0, 1], port);
        let token = child.clone();

        match warp::serve(filter)
            .try_bind_with_graceful_shutdown(address, async move { token.cancelled().await })
        {
            Ok((socket, server)) => Some((socket, server)),
            Err(_) => None,
        }
    });

    tokio::pin!(stream);
    let (socket, server) = stream
        .next()
        .await
        .expect("no available port in permitted range");

    let _guard = parent.drop_guard();
    tokio::spawn(server);

    let registry_index = resources.workspace().join("index");
    spawn_blocking({
        let registry_index = registry_index.clone();
        move || {
            let repo =
                Repository::init(&registry_index).expect("failed to initialise registry index");

            Stager::new(&repo)
                .add(b"config.json".to_vec(), {
                    let configuration = IndexFormat {
                        download: format!("http://127.0.0.1:{}", socket.port()),
                    };

                    serde_json::to_vec(&configuration)
                        .expect("failed to serialise index format")
                        .as_slice()
                })
                .add(
                    b"1/a".to_vec(),
                    r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to prepare registry index");

    let cache = resources.workspace().join("cache");
    let status = resources
        .exe()
        .create(
            &cache,
            &Url::from_file_path(registry_index).expect("failed to get url for registry index"),
        )
        .await;

    assert!(status.success(), "failed to create cache");
    assert_exists([&cache, &cache.join("index")].into_iter(), true).await;
    assert_exists([cache.join("crates")].into_iter(), false).await;

    for _ in 0..2 {
        let status = resources.exe().sync(&cache).await;
        assert!(status.success(), "failed to sync cache");
        assert_exists(
            [
                &cache,
                &cache.join("index"),
                &cache.join("crates"),
                &cache.join("crates/a/0.0.1/download"),
            ]
            .into_iter(),
            true,
        )
        .await;
    }
}

#[tokio::test]
async fn test_verify_with_consistent_cache() {
    let resources = Resources::new();

    let filter = warp::path!(String / String / "download").and_then(
        |name: String, version: String| async move {
            match (name.as_str(), version.as_str()) {
                ("a", "0.0.1") => Ok("0"),
                _ => Err(warp::reject::not_found()),
            }
        },
    );

    let parent = CancellationToken::new();
    let child = &parent.child_token();

    let stream = stream::iter(PERMITTED_PORTS).filter_map(|port| async move {
        let address = ([127, 0, 0, 1], port);
        let token = child.clone();

        match warp::serve(filter)
            .try_bind_with_graceful_shutdown(address, async move { token.cancelled().await })
        {
            Ok((socket, server)) => Some((socket, server)),
            Err(_) => None,
        }
    });

    tokio::pin!(stream);
    let (socket, server) = stream
        .next()
        .await
        .expect("no available port in permitted range");

    let _guard = parent.drop_guard();
    tokio::spawn(server);

    let registry_index = resources.workspace().join("index");
    spawn_blocking({
        let registry_index = registry_index.clone();
        move || {
            let repo =
                Repository::init(&registry_index).expect("failed to initialise registry index");

            Stager::new(&repo)
                .add(b"config.json".to_vec(), {
                    let configuration = IndexFormat {
                        download: format!("http://127.0.0.1:{}", socket.port()),
                    };

                    serde_json::to_vec(&configuration)
                        .expect("failed to serialise index format")
                        .as_slice()
                })
                .add(
                    b"1/a".to_vec(),
                    r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to prepare registry index");

    let cache = resources.workspace().join("cache");
    let status = resources
        .exe()
        .create(
            &cache,
            &Url::from_file_path(registry_index).expect("failed to get url for registry index"),
        )
        .await;

    assert!(status.success(), "failed to create cache");
    assert_exists([&cache, &cache.join("index")].into_iter(), true).await;
    assert_exists([cache.join("crates")].into_iter(), false).await;

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;
}

#[tokio::test]
async fn test_verify_with_empty_cache() {
    let resources = Resources::new();

    let filter = warp::path!(String / String / "download").and_then(
        |name: String, version: String| async move {
            match (name.as_str(), version.as_str()) {
                ("a", "0.0.1") => Ok("0"),
                _ => Err(warp::reject::not_found()),
            }
        },
    );

    let parent = CancellationToken::new();
    let child = &parent.child_token();

    let stream = stream::iter(PERMITTED_PORTS).filter_map(|port| async move {
        let address = ([127, 0, 0, 1], port);
        let token = child.clone();

        match warp::serve(filter)
            .try_bind_with_graceful_shutdown(address, async move { token.cancelled().await })
        {
            Ok((socket, server)) => Some((socket, server)),
            Err(_) => None,
        }
    });

    tokio::pin!(stream);
    let (socket, server) = stream
        .next()
        .await
        .expect("no available port in permitted range");

    let _guard = parent.drop_guard();
    tokio::spawn(server);

    let registry_index = resources.workspace().join("index");
    spawn_blocking({
        let registry_index = registry_index.clone();
        move || {
            let repo =
                Repository::init(&registry_index).expect("failed to initialise registry index");

            Stager::new(&repo)
                .add(b"config.json".to_vec(), {
                    let configuration = IndexFormat {
                        download: format!("http://127.0.0.1:{}", socket.port()),
                    };

                    serde_json::to_vec(&configuration)
                        .expect("failed to serialise index format")
                        .as_slice()
                })
                .add(
                    b"1/a".to_vec(),
                    r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to prepare registry index");

    let cache = resources.workspace().join("cache");
    let status = resources
        .exe()
        .create(
            &cache,
            &Url::from_file_path(registry_index).expect("failed to get url for registry index"),
        )
        .await;

    assert!(status.success(), "failed to create cache");
    assert_exists([&cache, &cache.join("index")].into_iter(), true).await;
    assert_exists([cache.join("crates")].into_iter(), false).await;

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;
}

#[tokio::test]
async fn test_verify_with_missing_cache() {
    let resources = Resources::new();

    let filter = warp::path!(String / String / "download").and_then(
        |name: String, version: String| async move {
            match (name.as_str(), version.as_str()) {
                ("a", "0.0.1") => Ok("0"),
                _ => Err(warp::reject::not_found()),
            }
        },
    );

    let parent = CancellationToken::new();
    let child = &parent.child_token();

    let stream = stream::iter(PERMITTED_PORTS).filter_map(|port| async move {
        let address = ([127, 0, 0, 1], port);
        let token = child.clone();

        match warp::serve(filter)
            .try_bind_with_graceful_shutdown(address, async move { token.cancelled().await })
        {
            Ok((socket, server)) => Some((socket, server)),
            Err(_) => None,
        }
    });

    tokio::pin!(stream);
    let (socket, server) = stream
        .next()
        .await
        .expect("no available port in permitted range");

    let _guard = parent.drop_guard();
    tokio::spawn(server);

    let registry_index = resources.workspace().join("index");
    spawn_blocking({
        let registry_index = registry_index.clone();
        move || {
            let repo =
                Repository::init(&registry_index).expect("failed to initialise registry index");

            Stager::new(&repo)
                .add(b"config.json".to_vec(), {
                    let configuration = IndexFormat {
                        download: format!("http://127.0.0.1:{}", socket.port()),
                    };

                    serde_json::to_vec(&configuration)
                        .expect("failed to serialise index format")
                        .as_slice()
                })
                .add(
                    b"1/a".to_vec(),
                    r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to prepare registry index");

    let cache = resources.workspace().join("cache");
    let status = resources
        .exe()
        .create(
            &cache,
            &Url::from_file_path(registry_index).expect("failed to get url for registry index"),
        )
        .await;

    assert!(status.success(), "failed to create cache");
    assert_exists([&cache, &cache.join("index")].into_iter(), true).await;
    assert_exists([cache.join("crates")].into_iter(), false).await;

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;

    fs::remove_file(cache.join("crates/a/0.0.1/download"))
        .await
        .expect("failed to remove crate");

    let status = resources.exe().verify(&cache).await;
    assert!(status.success(), "failed to verify cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;
}

#[tokio::test]
async fn test_verify_with_corrupted_cache() {
    let resources = Resources::new();

    let filter = warp::path!(String / String / "download").and_then(
        |name: String, version: String| async move {
            match (name.as_str(), version.as_str()) {
                ("a", "0.0.1") => Ok("0"),
                _ => Err(warp::reject::not_found()),
            }
        },
    );

    let parent = CancellationToken::new();
    let child = &parent.child_token();

    let stream = stream::iter(PERMITTED_PORTS).filter_map(|port| async move {
        let address = ([127, 0, 0, 1], port);
        let token = child.clone();

        match warp::serve(filter)
            .try_bind_with_graceful_shutdown(address, async move { token.cancelled().await })
        {
            Ok((socket, server)) => Some((socket, server)),
            Err(_) => None,
        }
    });

    tokio::pin!(stream);
    let (socket, server) = stream
        .next()
        .await
        .expect("no available port in permitted range");

    let _guard = parent.drop_guard();
    tokio::spawn(server);

    let registry_index = resources.workspace().join("index");
    spawn_blocking({
        let registry_index = registry_index.clone();
        move || {
            let repo =
                Repository::init(&registry_index).expect("failed to initialise registry index");

            Stager::new(&repo)
                .add(b"config.json".to_vec(), {
                    let configuration = IndexFormat {
                        download: format!("http://127.0.0.1:{}", socket.port()),
                    };

                    serde_json::to_vec(&configuration)
                        .expect("failed to serialise index format")
                        .as_slice()
                })
                .add(
                    b"1/a".to_vec(),
                    r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to prepare registry index");

    let cache = resources.workspace().join("cache");
    let status = resources
        .exe()
        .create(
            &cache,
            &Url::from_file_path(registry_index).expect("failed to get url for registry index"),
        )
        .await;

    assert!(status.success(), "failed to create cache");
    assert_exists([&cache, &cache.join("index")].into_iter(), true).await;
    assert_exists([cache.join("crates")].into_iter(), false).await;

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;

    fs::write(cache.join("crates/a/0.0.1/download"), "corrupted")
        .await
        .expect("failed to corrupt crate");

    let status = resources.exe().verify(&cache).await;
    assert!(status.success(), "failed to verify cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn test_update_with_package_addition() {
    let resources = Resources::new();

    let filter = warp::path!(String / String / "download").and_then(
        |name: String, version: String| async move {
            match (name.as_str(), version.as_str()) {
                ("a" | "b", "0.0.1") => Ok("0"),
                _ => Err(warp::reject::not_found()),
            }
        },
    );

    let parent = CancellationToken::new();
    let child = &parent.child_token();
    let stream = stream::iter(PERMITTED_PORTS).filter_map(|port| async move {
        let address = ([127, 0, 0, 1], port);
        let token = child.clone();

        match warp::serve(filter)
            .try_bind_with_graceful_shutdown(address, async move { token.cancelled().await })
        {
            Ok((socket, server)) => Some((socket, server)),
            Err(_) => None,
        }
    });

    tokio::pin!(stream);
    let (socket, server) = stream
        .next()
        .await
        .expect("no available port in permitted range");

    let _guard = parent.drop_guard();
    tokio::spawn(server);

    let registry_index = resources.workspace().join("index");
    spawn_blocking({
        let registry_index = registry_index.clone();
        move || {
            let repo =
                Repository::init(&registry_index).expect("failed to initialise registry index");

            Stager::new(&repo)
                .add(b"config.json".to_vec(), {
                    let configuration = IndexFormat {
                        download: format!("http://127.0.0.1:{}", socket.port()),
                    };

                    serde_json::to_vec(&configuration)
                        .expect("failed to serialise index format")
                        .as_slice()
                })
                .add(
                    b"1/a".to_vec(),
                    r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to prepare registry index");

    let cache = resources.workspace().join("cache");
    let status = resources
        .exe()
        .create(
            &cache,
            &Url::from_file_path(&registry_index).expect("failed to get url for registry index"),
        )
        .await;

    assert!(status.success(), "failed to create cache");
    assert_exists([&cache, &cache.join("index")].into_iter(), true).await;
    assert_exists([cache.join("crates")].into_iter(), false).await;

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;

    spawn_blocking({
        move || {
            let repo = Repository::open(&registry_index).expect("failed to open registry index");
            Stager::new(&repo)
                .add(
                    b"1/b".to_vec(),
                    r#"{"name":"b","vers":"0.0.1","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to add crate to registry index");

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
            &cache.join("crates/b/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn test_update_with_crate_addition() {
    let resources = Resources::new();

    let filter = warp::path!(String / String / "download").and_then(
        |name: String, version: String| async move {
            match (name.as_str(), version.as_str()) {
                ("a", "0.0.1" | "0.0.2") => Ok("0"),
                _ => Err(warp::reject::not_found()),
            }
        },
    );

    let parent = CancellationToken::new();
    let child = &parent.child_token();
    let stream = stream::iter(PERMITTED_PORTS).filter_map(|port| async move {
        let address = ([127, 0, 0, 1], port);
        let token = child.clone();

        match warp::serve(filter)
            .try_bind_with_graceful_shutdown(address, async move { token.cancelled().await })
        {
            Ok((socket, server)) => Some((socket, server)),
            Err(_) => None,
        }
    });

    tokio::pin!(stream);
    let (socket, server) = stream
        .next()
        .await
        .expect("no available port in permitted range");

    let _guard = parent.drop_guard();
    tokio::spawn(server);

    let registry_index = resources.workspace().join("index");
    spawn_blocking({
        let registry_index = registry_index.clone();
        move || {
            let repo =
                Repository::init(&registry_index).expect("failed to initialise registry index");

            Stager::new(&repo)
                .add(b"config.json".to_vec(), {
                    let configuration = IndexFormat {
                        download: format!("http://127.0.0.1:{}", socket.port()),
                    };

                    serde_json::to_vec(&configuration)
                        .expect("failed to serialise index format")
                        .as_slice()
                })
                .add(
                    b"1/a".to_vec(),
                    r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to prepare registry index");

    let cache = resources.workspace().join("cache");
    let status = resources
        .exe()
        .create(
            &cache,
            &Url::from_file_path(&registry_index).expect("failed to get url for registry index"),
        )
        .await;

    assert!(status.success(), "failed to create cache");
    assert_exists([&cache, &cache.join("index")].into_iter(), true).await;
    assert_exists([cache.join("crates")].into_iter(), false).await;

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;

    spawn_blocking({
        move || {
            let repo = Repository::open(&registry_index).expect("failed to open registry index");
            Stager::new(&repo)
                .add(
                    b"1/a".to_vec(),
                    r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}
{"name":"a","vers":"0.0.2","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to add crate to registry index");

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
            &cache.join("crates/a/0.0.2/download"),
        ]
        .into_iter(),
        true,
    )
    .await;
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn test_update_with_crate_modification() {
    let resources = Resources::new();

    let parent = CancellationToken::new();
    let child = &parent.child_token();

    let crate_data = Arc::new(Mutex::new(String::from("0")));
    let stream = stream::iter(PERMITTED_PORTS).filter_map(|port| {
        let crate_data = crate_data.clone();
        async move {
            let address = ([127, 0, 0, 1], port);
            let token = child.clone();

            let filter = warp::path!(String / String / "download").and_then(
                move |name: String, version: String| {
                    let crate_data = crate_data.clone();
                    async move {
                        match (name.as_str(), version.as_str()) {
                            ("a", "0.0.1") => {
                                Ok(crate_data.lock().expect("lock is poisoned").clone())
                            }
                            _ => Err(warp::reject::not_found()),
                        }
                    }
                },
            );

            match warp::serve(filter)
                .try_bind_with_graceful_shutdown(address, async move { token.cancelled().await })
            {
                Ok((socket, server)) => Some((socket, server)),
                Err(_) => None,
            }
        }
    });

    tokio::pin!(stream);
    let (socket, server) = stream
        .next()
        .await
        .expect("no available port in permitted range");

    let _guard = parent.drop_guard();
    tokio::spawn(server);

    let registry_index = resources.workspace().join("index");
    spawn_blocking({
        let registry_index = registry_index.clone();
        move || {
            let repo =
                Repository::init(&registry_index).expect("failed to initialise registry index");

            Stager::new(&repo)
                .add(b"config.json".to_vec(), {
                    let configuration = IndexFormat {
                        download: format!("http://127.0.0.1:{}", socket.port()),
                    };

                    serde_json::to_vec(&configuration)
                        .expect("failed to serialise index format")
                        .as_slice()
                })
                .add(
                    b"1/a".to_vec(),
                    r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to prepare registry index");

    let cache = resources.workspace().join("cache");
    let status = resources
        .exe()
        .create(
            &cache,
            &Url::from_file_path(&registry_index).expect("failed to get url for registry index"),
        )
        .await;

    assert!(status.success(), "failed to create cache");
    assert_exists([&cache, &cache.join("index")].into_iter(), true).await;
    assert_exists([cache.join("crates")].into_iter(), false).await;

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;

    spawn_blocking({
        move || {
            let repo = Repository::open(&registry_index).expect("failed to open registry index");
            Stager::new(&repo)
                .add(
                    b"1/a".to_vec(),
                    r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"938db8c9f82c8cb58d3f3ef4fd250036a48d26a712753d2fde5abd03a85cabf4","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to add crate to registry index");
    crate_data.lock().expect("lock poisoned").push('1');

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;

    // Ensure that the new version was downloaded.
    assert_eq!(
        fs::read_to_string(cache.join("crates/a/0.0.1/download"))
            .await
            .expect("failed to read from cache"),
        String::from("01")
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn test_update_with_crate_removal() {
    let resources = Resources::new();

    let filter = warp::path!(String / String / "download").and_then(
        |name: String, version: String| async move {
            match (name.as_str(), version.as_str()) {
                ("a", "0.0.1") => Ok("0"),
                _ => Err(warp::reject::not_found()),
            }
        },
    );

    let parent = CancellationToken::new();
    let child = &parent.child_token();
    let stream = stream::iter(PERMITTED_PORTS).filter_map(|port| async move {
        let address = ([127, 0, 0, 1], port);
        let token = child.clone();

        match warp::serve(filter)
            .try_bind_with_graceful_shutdown(address, async move { token.cancelled().await })
        {
            Ok((socket, server)) => Some((socket, server)),
            Err(_) => None,
        }
    });

    tokio::pin!(stream);
    let (socket, server) = stream
        .next()
        .await
        .expect("no available port in permitted range");

    let _guard = parent.drop_guard();
    tokio::spawn(server);

    let registry_index = resources.workspace().join("index");
    spawn_blocking({
        let registry_index = registry_index.clone();
        move || {
            let repo =
                Repository::init(&registry_index).expect("failed to initialise registry index");

            Stager::new(&repo)
                .add(b"config.json".to_vec(), {
                    let configuration = IndexFormat {
                        download: format!("http://127.0.0.1:{}", socket.port()),
                    };

                    serde_json::to_vec(&configuration)
                        .expect("failed to serialise index format")
                        .as_slice()
                })
                .add(
                    b"1/a".to_vec(),
                    r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9","features":{},"yanked":false}"#.as_bytes()
                )
                .commit();
        }
    })
    .await
    .expect("failed to prepare registry index");

    let cache = resources.workspace().join("cache");
    let status = resources
        .exe()
        .create(
            &cache,
            &Url::from_file_path(&registry_index).expect("failed to get url for registry index"),
        )
        .await;

    assert!(status.success(), "failed to create cache");
    assert_exists([&cache, &cache.join("index")].into_iter(), true).await;
    assert_exists([cache.join("crates")].into_iter(), false).await;

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists(
        [
            &cache,
            &cache.join("index"),
            &cache.join("crates"),
            &cache.join("crates/a/0.0.1/download"),
        ]
        .into_iter(),
        true,
    )
    .await;

    spawn_blocking({
        move || {
            let repo = Repository::open(&registry_index).expect("failed to open registry index");
            Stager::new(&repo).remove(Path::new("1/a")).commit();
        }
    })
    .await
    .expect("failed to add crate to registry index");

    let status = resources.exe().sync(&cache).await;
    assert!(status.success(), "failed to sync cache");
    assert_exists([&cache, &cache.join("index")].into_iter(), true).await;
    // There are no crates. Obsolete directories should be removed.
    assert_exists([&cache.join("cache")].into_iter(), false).await;
}
