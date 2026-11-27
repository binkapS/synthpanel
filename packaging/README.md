# Packaging

## Debian / Ubuntu

```sh
cargo build --release --workspace
VERSION=0.2.0 packaging/build-deb.sh
sudo dpkg -i target/debian/synthpanel_0.2.0_amd64.deb
```

The package installs the four binaries under `/usr/local/bin`, the systemd
units under `/lib/systemd/system`, creates the `synthpanel` system user and
the state directories, and enables both services. No environment variables are
required by the `.deb`; every override below is optional and has a built-in
default.

## Optional environment overrides

- `SYNTHPANEL_EXTRA_PACKAGES` — extra apt packages for the installer (space or
  comma separated). The base package list is installed without it.
- `SYNTHPANEL_PHP_FPM_8_5` (also `_8_4`, `_8_3`, …) — absolute path to a
  version's FPM binary when it is not on `PATH`. Without it the panel searches
  both `php-fpm85` and `php-fpm8.5` on `PATH` and reports the version as not
  installed when neither exists.
- `SYNTHPANEL_PHP_DIR` — base directory for generated PHP-FPM configuration;
  defaults to `/etc/synthpanel/php`.

## apt repository

`packaging/apt/publish.sh` builds a flat repository layout suitable for
`apt.synthpanel.io` (or any static host):

```sh
packaging/apt/publish.sh target/debian/*.deb
# optional signing
GPG_KEY=release@synthpanel.io packaging/apt/publish.sh target/debian/*.deb
```

Serve `target/apt-repo` over HTTPS and add it on clients with:

```sh
curl -fsSL https://apt.synthpanel.io/key.gpg | sudo gpg --dearmor -o /usr/share/keyrings/synthpanel.gpg
echo "deb [signed-by=/usr/share/keyrings/synthpanel.gpg] https://apt.synthpanel.io stable main" \
  | sudo tee /etc/apt/sources.list.d/synthpanel.list
sudo apt update && sudo apt install synthpanel
```

## Offline

The release tarball contains the four binaries; `synthpanel-installer
--skip-packages --offline` installs them without network access.

## Self-update

Releases are published as a signed envelope (`payload` + Ed25519
`signature`). Set the trusted key on the server:

```sh
SYNTHPANEL_UPDATE_PUBKEY=<hex key> synthpanel update --check
```

The core downloads the archive, verifies the signature and SHA-256, backs up
the current binaries, replaces them, restarts the units, runs a health check
and rolls back automatically when anything fails.
