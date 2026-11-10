# SynthPanel

A free, open-source LEMP control panel for a single Linux server.
Written in Rust + Svelte 5, targeting Ubuntu LTS and Debian 12+.

See [PLAN.md](./PLAN.md) for the full design and
[docs/roadmap.md](./docs/roadmap.md) for what is implemented today.

## Development

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd crates/synthpanel-web/frontend && npm install && npm run build
```

## License

AGPL-3.0-or-later.
