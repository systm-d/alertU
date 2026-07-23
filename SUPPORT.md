# Getting help

- **A bug, an idea, or a remote that will not work?** Open an issue — the
  [templates](https://github.com/systm-d/alertU/issues/new/choose) walk you
  through what to include.
- **Not sure a behaviour is a bug?** Check the configuration reference in
  [`packaging/config.example.toml`](packaging/config.example.toml); every field
  is documented inline.
- **A security problem?** Do not open an issue — follow
  [`SECURITY.md`](SECURITY.md) and report it privately.

AlertU is a personal project maintained in spare time, so answers are best
effort. The fastest route to a fix is a clear report with your version
(`alertu-ctl --version`), your distribution, and the daemon's logs
(`sudo journalctl -u alertu-daemon`).
