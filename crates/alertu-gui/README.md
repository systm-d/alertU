# alertu-gui

The [AlertU](https://github.com/systm-d/alertU) tray: a StatusNotifierItem
(via pure-Rust `ksni`, no libdbus) that reflects the guard's state, lets you pick
the remote and the watched devices, and survives the daemon restarting — it
reconnects with exponential backoff rather than disappearing.

Needs a running `alertu-daemon` to talk to, and a StatusNotifierItem host in your
desktop environment.

## License

MIT — see [LICENSE](https://github.com/systm-d/alertU/blob/main/LICENSE).
