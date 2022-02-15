#![warn(clippy::all, clippy::cargo, clippy::nursery, clippy::pedantic)]
#![allow(clippy::multiple_crate_versions)]

mod digest;
mod download;
mod registry;

use clap::{Parser, Subcommand};
use eyre::Result;
use registry::cache::Cache;
use reqwest::{Client, ClientBuilder};
use std::{num::NonZeroUsize, path::PathBuf};
use tracing::info;
use url::Url;

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

async fn new(path: PathBuf, url: Url) -> Result<()> {
    drop(Cache::new(path, url).await?);
    info!("created cache");

    Ok(())
}

async fn verify(path: PathBuf, jobs: NonZeroUsize, client: &Client) -> Result<()> {
    let cache = Cache::from_path(path).await?;
    let options = download::Options {
        preserve: download::PreservationStrategy::Checksum,
    };

    cache.refresh(client, options, jobs).await?;
    info!("verified cache");

    Ok(())
}

async fn synchronise(path: PathBuf, jobs: NonZeroUsize, client: &Client) -> Result<()> {
    let cache = Cache::from_path(path).await?;
    let options = download::Options::default();

    cache.refresh(client, options, jobs).await?;
    info!("refreshed cache");

    cache.update(client, options, jobs).await?;
    info!("updated cache");
    info!("cache is synchronised");

    Ok(())
}

/// Collects the program arguments
#[derive(Parser, Debug)]
#[clap(version, about)]
struct Arguments {
    #[clap(subcommand)]
    action: Action,

    /// The path of the registry cache
    #[clap(short, long)]
    path: PathBuf,

    /// The number of jobs that can run in parallel
    #[clap(short, long, default_value_t = NonZeroUsize::new(1).unwrap())]
    jobs: NonZeroUsize,

    /// The log level to use
    #[clap(short, long, default_value_t = tracing::Level::INFO)]
    log_level: tracing::Level,

    /// Contact information for the user
    ///
    /// Some registries have a policy that asks crawlers to provide contact information. This
    /// information is transmitted in the user agent of HTTP requests.
    #[clap(short, long)]
    contact: Option<String>,
}

/// Represents an action that a user requests.
#[derive(Debug, Subcommand)]
enum Action {
    /// Creates a new cache.
    #[clap(name = "new")]
    New {
        /// The URL of the index.
        #[clap(short, long)]
        url: Url,
    },

    /// Verifies the integrity of the cache and (re)downloads any corrupt or missing crates.
    #[clap(name = "verify")]
    Verify,

    /// Synchronises a cache.
    #[clap(name = "sync")]
    Synchronise,
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();

    tracing_subscriber::fmt()
        .with_max_level(arguments.log_level)
        .init();

    match arguments.action {
        Action::New { url } => new(arguments.path, url).await,
        action => {
            let mut builder = ClientBuilder::new();
            builder = match arguments.contact {
                Some(contact) => builder.user_agent(format!("{} ({})", USER_AGENT, contact)),
                None => builder.user_agent(USER_AGENT),
            };
            let client = builder.build()?;

            match action {
                Action::Verify => verify(arguments.path, arguments.jobs, &client).await,
                Action::Synchronise => synchronise(arguments.path, arguments.jobs, &client).await,

                // Already covered.
                Action::New { url: _ } => unreachable!(),
            }
        }
    }
}
