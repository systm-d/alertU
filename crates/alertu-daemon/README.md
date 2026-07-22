# alertu-daemon

The privileged half of [AlertU](https://github.com/systm-d/alertU): it reads
evdev devices, runs the guard state machine, locks and unlocks the desktop
session through logind, plays sounds, captures webcam stills, and serves the
control socket every front end talks to.

Its transition table is a pure function with no I/O, interpreted separately, so
the alarm's logic is exhaustively unit-tested.

Installing this crate alone is not enough to run AlertU: it needs a systemd
unit, a service account in the `input` and `video` groups, and sound files. See
the [project README](https://github.com/systm-d/alertU) for the install steps.

## License

MIT — see [LICENSE](https://github.com/systm-d/alertU/blob/main/LICENSE).
