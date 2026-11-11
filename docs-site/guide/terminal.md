# Terminal

**Terminal** opens a shell on the server.

The shell runs inside the privileged core, in its own pseudo-terminal, and the
browser talks to it over a WebSocket. Only administrators can open a
session, and every session is closed when the page is left or after 30
minutes of inactivity.

## Why there is no SSH port

The panel exposes one port: its own HTTPS interface. The terminal is part of
that interface, so there is nothing else to open in the firewall and no
second credential to manage. Sessions live in the core, so the unprivileged
web process never needs shell access of its own.

## Notes

- The session inherits the environment of the core, including the
  `SYNTHPANEL_*` paths.
- Terminal sessions are not recorded in the audit log beyond their creation
  and closing; run `synthpanel audit` to see them.
- Interactive programs that need a second TTY (`sudo` password prompts,
  `ssh` with a passphrase) do not work.
