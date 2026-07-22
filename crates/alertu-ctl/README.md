# alertu-ctl

Command-line control for [AlertU](https://github.com/systm-d/alertU) — everything
the tray does, from a shell or a script.

```sh
alertu-ctl status              # Idle | Armed | Triggered | Alarm
alertu-ctl arm                 # force-arm (locks the session)
alertu-ctl toggle              # exactly what a remote click does
alertu-ctl get-config          # the daemon's effective config, as TOML
alertu-ctl list-devices        # the input devices the daemon can see
alertu-ctl gen-sounds --dir /usr/share/sounds/alertu

alertu-ctl --json status       # {"event":"state","state":"idle"}
alertu-ctl status --watch      # one line per state change
```

`--json` prints the daemon's raw protocol response. Exit codes: `0` success,
`1` daemon or connection error, `2` usage error.

Needs a running `alertu-daemon`, and your login must be in the group that owns
its socket.

## License

MIT — see [LICENSE](https://github.com/systm-d/alertU/blob/main/LICENSE).
