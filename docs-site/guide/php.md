# PHP

**PHP** shows what the panel knows about the PHP installation of the server:
the versions it supports, the FPM binary of each version, the pools the panel
generated and the sites that use them.

## Per-site pools

Every site gets its own PHP-FPM pool, named after the site id:

```
/etc/synthpanel/php/8.3/fpm/pool.d/synthpanel-<site-id>.conf
```

The pool user is `sp_<site-id>` and the socket lives in `/run/php`. A slow
site therefore can never starve the others.

## Changing the version of a site

Open the site and pick another PHP version. The panel regenerates the pool,
writes the new vhost and reloads the service — and rolls the change back when
the configuration does not validate.

## Versions the panel supports

| Version | Status |
| --- | --- |
| 8.1 | supported |
| 8.2 | supported |
| 8.3 | supported (the default) |
| 8.4 | supported |

The page marks a version as *not installed* when its `php-fpm` binary is not
on the PATH, and *installed, but not running* when the service is inactive.
