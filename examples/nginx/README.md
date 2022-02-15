# NGINX

## Installation

How *NGINX* configuration files should be installed depends on user requirements and the operating
system. On Linux, however, they can generally be installed to `/etc/nginx/conf.d` which is
automatically included by the default configuration file.

The configurations were written for [Fedora](https://getfedora.org/) and may need to be modified
slightly to run on other operating systems.

## SELinux

Generally, when SELinux is enabled and enforcing policies, *NGINX* will not be able to serve files
that are not correctly labelled. The cache directory and its contents should be laballed with
`httpd_sys_content_t` or `httpd_user_content_t`.

## Registry

`registry.conf` is a configuration for hosting a <crates.io> registry mirror. It defines a virtual
host and maps requested paths to the cached registry.

## Index

`index.conf` is a configuration for hosting a companion <crates.io> registry index mirror. *NGINX*
is not strictly required. The index can be hosted by [GitLab](https://gitlab.com) or any other
service capable of serving git repositories.

### Dependencies

The configuration uses `git-http-backend` and `fcgiwrap`. These programs must be installed for
*NGINX* to serve the registry index.
