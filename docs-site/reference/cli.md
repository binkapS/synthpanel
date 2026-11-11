# CLI reference

The `synthpanel` CLI talks to the core over `/run/synthpanel/core.sock` and
uses the token at `/etc/synthpanel/ipc-token`. Run it with `sudo` on the server.

```text
synthpanel status
synthpanel metrics
synthpanel metrics:range --range 24h
synthpanel audit --limit 25
synthpanel settings

synthpanel sites:list
synthpanel sites:add example.com --aliases www.example.com --template php
synthpanel sites:show <id>
synthpanel sites:delete <id>

synthpanel users:list
synthpanel users:create you@example.com --name You --role admin

synthpanel db:list
synthpanel db:create app --engine mysql
synthpanel db:delete <id>

synthpanel security:firewall
synthpanel security:fail2ban
synthpanel security:ssh-keys

synthpanel backup:list
synthpanel backup:run [--site <id>] [--passphrase <secret>]
synthpanel backup:verify <id>
synthpanel backup:restore <id>

synthpanel cron:list
synthpanel cron:add "0 3 * * *" "php artisan schedule:run" --user www-data
synthpanel cron:run <id>
synthpanel cron:runs <id>

synthpanel logs --type panel --search error
synthpanel files:list --scope www --path site/
synthpanel files:read --scope www site/index.php
synthpanel files:write --scope www site/index.php --file index.php

synthpanel apikeys:list
synthpanel apikeys:create "CI deploy" --user <user-id> --scopes read,write
synthpanel apikeys:delete <id>

synthpanel update --check
synthpanel update:status
synthpanel update:rollback --id <update-id>
```
