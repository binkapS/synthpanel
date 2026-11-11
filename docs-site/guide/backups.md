# Backups & restore

A backup is a single `.tar.gz` containing:

- generated Nginx vhosts
- generated PHP-FPM pools
- site document roots
- database dumps
- a `synthpanel-manifest.json` describing the contents

## Destinations

- **Local** — default `/var/backups/synthpanel/archives`
- **SFTP** — host, port, user, key and remote directory
- **S3-compatible** — endpoint, bucket, region, access key and secret
  (AWS SigV4, works with MinIO)

## Encryption

Pass a passphrase when running a backup. The archive is encrypted with
ChaCha20-Poly1305 before it leaves the server. Restore and download require the
same passphrase.

## Verification

`backups.verify` (UI: the check icon) unpacks the archive into a staging
directory and validates the manifest and checksum without touching live files.

## Restore

`backups.restore` writes the vhosts, pools and document roots back. A site
backup records the original document root so a custom root keeps its place.
Restoring does **not** recreate a deleted `sites` row; re-create the site first
if you want it managed again.
