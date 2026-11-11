# Quickstart

1. **Add a site.** Sites → Add site. Each domain gets its own Nginx vhost, a
   dedicated PHP-FPM pool and an optional Let's Encrypt certificate.
2. **Create a database.** Databases → New database. The panel provisions the
   database and user on the server first, then records them.
3. **Run a backup.** Backups → Run backup now. Archives include vhosts, pools,
   document roots and database dumps.
4. **Schedule it.** Add a backup schedule with a cron expression and a
   retention count.
5. **Invite your team.** Users → New user. Roles are admin, operator and user;
   quotas limit sites and databases per account.

Every action is audit-logged and available through the API and CLI:

```sh
synthpanel sites:list
synthpanel db:create app --engine mysql
synthpanel backup:run
synthpanel logs --type nginx-error
```
