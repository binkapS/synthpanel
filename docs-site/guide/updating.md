# Updating

SynthPanel updates are signed. The server trusts one Ed25519 public key:

```sh
export SYNTHPANEL_UPDATE_PUBKEY=<64 hex characters>
```

## Check

```sh
synthpanel update --check
```

or **Settings → Panel update → Check for updates**.

## Apply

```sh
synthpanel update
```

The core:

1. downloads the release archive,
2. verifies the manifest signature and the archive SHA-256,
3. copies the current binaries into a rollback directory,
4. replaces the binaries,
5. restarts `synthpanel-core` and `synthpanel-web`,
6. runs a health check,
7. rolls back automatically if the health check or restart fails.

## Rollback

```sh
synthpanel update:status
synthpanel update:rollback --id <update-id>
```
