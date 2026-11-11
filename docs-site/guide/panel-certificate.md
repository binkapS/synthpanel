# The panel certificate

The panel serves HTTPS on port 8443. On the first start it generates a
self-signed certificate, which the browser flags. To get a trusted
certificate, request one from Let's Encrypt.

## Before you start

1. Set the **hostname** in **Settings → General** to the name your browser
   uses, e.g. `panel.example.com`.
2. Make sure the name resolves to this server.
3. Make sure port 80 reaches the panel:
   - either forward port 80 to the panel, or
   - let a proxy forward `/.well-known/acme-challenge/` to the panel.
4. Install acme.sh (the panel uses it for every certificate):

   ```sh
   curl https://get.acme.sh | sh
   ```

## Requesting the certificate

**Settings → Panel certificate → Request certificate**.

Two methods are available:

| Method | What it does |
| --- | --- |
| `webroot` | acme.sh writes the challenge into `/var/lib/synthpanel/acme-webroot`, and the panel serves it over `/.well-known/acme-challenge/` |
| `standalone` | acme.sh binds port 80 itself; port 80 has to be free |

The certificate is installed as `panel-cert.pem` and `panel-key.pem` in the
TLS directory. The panel picks it up on the next connection — no restart is
needed, but reload the browser to see it.

## Behind a proxy

When a proxy terminates TLS and forwards plain HTTP, the panel has to know
the original scheme, otherwise the browser discards the session cookie. Send
this header from the proxy:

```
X-Forwarded-Proto: $scheme
```
