# Installation

SynthPanel targets Debian 12 and Ubuntu 22.04/24.04 on a fresh server.

## Requirements

- A fresh x86-64 or arm64 server with root access
- A hostname pointing at the server (needed before issuing a Let's Encrypt
  certificate for the panel itself)
- Ports 80 and 443 reachable from the internet

## One-liner

```sh
curl -sSfL https://get.synthpanel.io | sudo bash
```

The installer detects the distribution, installs Nginx, PHP-FPM, MariaDB,
PostgreSQL and Redis, creates the `synthpanel` system user, writes the systemd
units, applies a firewall baseline, and bootstraps an admin account.

## From a package

```sh
sudo dpkg -i synthpanel_0.1.0_amd64.deb
```

or, once the apt repository is configured:

```sh
sudo apt install synthpanel
```

## Offline

The release tarball contains every binary. On an air-gapped host:

```sh
sudo synthpanel-installer --unattended --skip-packages --offline \
  --hostname panel.example.com --admin-email you@example.com
```

## After installation

Open `https://<hostname>:8443` and sign in with the admin account. The panel
generates a self-signed certificate on first start; issue a trusted
certificate from **Settings → Hostname** once DNS resolves.
