# alertu-common

Shared types for [AlertU](https://github.com/systm-d/alertU): the TOML
configuration, the guard state enum, and the newline-delimited JSON protocol
spoken over the daemon's Unix socket.

The `ipc-client` feature adds a small blocking client for that socket. It is off
by default, because the daemon runs its own async server and has no use for it.

See the [project README](https://github.com/systm-d/alertU) for what AlertU does
and how the pieces fit together.

## License

MIT — see [LICENSE](https://github.com/systm-d/alertU/blob/main/LICENSE).
