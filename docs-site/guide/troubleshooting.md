# Troubleshooting

## The panel does not start

```sh
systemctl status synthpanel-core synthpanel-web
journalctl -u synthpanel-core -n 100 --no-pager
```

The web process needs the IPC token written by the core at
`/etc/synthpanel/ipc-token`. Start the core first.

## A site change broke nginx

The apply pipeline runs `nginx -t` before reloading and restores the previous
vhost when validation fails. Check the panel log for the rollback message:

```sh
synthpanel logs --type panel --search rollback
```

## A certificate was not issued

Let's Encrypt uses HTTP-01, so the domain must resolve to the server and port
80 must be reachable. Check the issuance output in the site detail page and the
`acme.sh` log under `/var/log/synthpanel`.

## Emergency access

If the web UI is down, the CLI talks to the core over the Unix socket:

```sh
sudo synthpanel status
sudo synthpanel sites:list
sudo synthpanel users:create you@example.com --role admin
```
