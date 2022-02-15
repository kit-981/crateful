# crateful

*crateful* is a command line program that can be used to cache [Cargo package
registries](https://doc.rust-lang.org/cargo/reference/registries.html). *crateful* is modular and
unopinionated by design and can be composed with other tools to provide a fully-fledged offline Rust
development environment.

## Features

- Recovers from file system and network failures
- Supports alternative *Cargo* package registries

## Usage

*crateful* requires a private and dedicated directory along with the url of the index for the
package registry cache.

```
$ crateful --path /path/to/cache new --url http://link/to/index
$ crateful --path /path/to/cache sync
```

Temporary file system errors (eg. not enough disk space) or network failures (eg. internet outages)
are recoverable by running the command again until it's successful. If an operation fails, the cache
may be left in an inconsistent state and it should not be used until the command runs successfully.

By default, *crateful* only performs integrity checking before and after downloading a file. This is
a performance optimisation. However, *crateful* can verify the state of the cache if corruption is
suspected.

```
$ crateful --path /path/to/cache verify
```

Verifying a cache may correct unexpected modifications and deletions but the operation will not
remove files that are not tracked by the index.

### Performance

It is strongly recommended to use the `jobs` argument for operations that support it. This argument
configures the number of actions that `crateful` will perform in parallel. This will reduce the time
operations require to complete.

```
$ crateful --path /path/to/cache --jobs 4 sync
```

### Mirroring

A cache created by *crateful* contains two directories. The `crates` directory is structured to
match the default crate download locations. It can be statically hosted by a web server. The `index`
directory is a clone of the registry index repository that was provided at creation-time. It should
be copied and it's [registry index
format](https://doc.rust-lang.org/cargo/reference/registries.html#index-format) must be modified
with the details of the web server location. This change must be committed. The index can be hosted
by a web server that supports [Git](https://git-scm.com/book/en/v2/Git-on-the-Server-The-Protocols).

The registry mirror will be ready to use following the above instructions and details on how to use
can be found in the [Cargo
reference](https://doc.rust-lang.org/cargo/reference/registries.html#using-an-alternate-registry).

#### Examples

Example configurations for [NGINX](https://www.nginx.com/) and [systemd](https://systemd.io/) are
available in `/examples`. It is strongly recommended to review the Cargo reference documentation on
registries before using the configurations.

## License

[GPL version 3](https://www.gnu.org/licenses/gpl-3.0.en.html) or later
