# Deployments

A deployment connects a site to a Git repository. The panel clones or updates
the repository, runs an optional build command and publishes the result into
the site's document root.

## Creating a deployment

**Deployments → Add deployment**, or:

```sh
synthpanel deploy:add --site <site-id> --repo https://github.com/example/app.git \
  --branch main --subdir public --build "npm ci && npm run build" --user www-data
```

| Field | Meaning |
| --- | --- |
| Repository | `https://`, `ssh://`, `git@` or `file://` URL |
| Branch | The branch that is deployed |
| Sub-directory | Published when the repository has more than one project |
| Build command | Runs in the checkout before publishing |
| Deploy user | The user the build runs as and the files belong to |

## How a deployment runs

1. The repository is cloned (or fetched and reset) into
   `/var/lib/synthpanel/deploy/<id>/checkout`.
2. The build command runs in the checkout, as the deploy user.
3. The result is copied to `/var/lib/synthpanel/deploy/<id>/releases/<commit>`.
4. The current document root is copied to `…/previous` and the new release
   replaces it with an atomic directory swap.
5. The files are handed to the deploy user.

If any step fails, the previous release is published again and the run is
recorded as `failed`. Every run is listed under **Runs** with its commit, the
command output and the error.

## Rolling back

Re-run the deployment with the previous commit, or restore the site from a
backup. The `…/previous` directory always holds the release that was live
before the last deployment.
