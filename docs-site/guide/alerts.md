# Alerts

Alerts let the panel tell you when something needs attention.

## Channels

A channel is where an alert is delivered.

- **Webhook** — the panel POSTs a JSON document to your URL. When a signing
  secret is configured, the payload is signed with HMAC-SHA256 and sent in
  the `X-SynthPanel-Signature` header.
- **Email** — the panel talks to an SMTP server (`AUTH PLAIN`/`AUTH LOGIN`,
  optional `STARTTLS`) or pipes the message into a local MTA such as
  Postfix.

```sh
synthpanel alerts:channel:add ops --kind webhook \
  --config '{"url":"https://hooks.example.com/xyz","secret":"…"}'
```

## Rules

A rule connects an event to a channel:

| Event | Raised when |
| --- | --- |
| `backup.failed` | A scheduled or manual backup fails |
| `backup.succeeded` | A scheduled backup succeeds |
| `cron.failed` | A cron job exits with a non-zero code |
| `deploy.failed` / `deploy.succeeded` | A deployment finishes |
| `ssl.expiring` | A certificate expires within 14 days |
| `disk.usage` | The disk is more than 85% full |
| `update.failed` | A self-update fails |

The **cooldown** of a rule (15 minutes by default) keeps a failing job from
flooding your channel. Every delivery — successful or not — is listed in the
history with its error message.

## Testing a channel

Use the send button next to a channel. The panel delivers a test alert and
reports the result, which is the fastest way to find a wrong URL or a
rejected SMTP login.
