# resources/

Source material for the repository, not an install directory.

**The sounds users install come from `alertu-ctl gen-sounds`, not from copying
these files.** The documented step is:

```sh
sudo alertu-ctl gen-sounds --dir /usr/share/sounds/alertu
```

which writes `beep.wav`, `warning.wav` and `siren.wav` with the right modes
(0644, in a 0755 directory) so the unprivileged `alertu` service account can
read them. Copying the files here by hand skips that and produces sounds the
daemon may silently fail to play.

| File | Role |
|------|------|
| `lock.wav` | The arm/disarm chirp. **Embedded into the `alertu-ctl` binary** at compile time (`include_bytes!` in `crates/alertu-ctl/src/main.rs`); `gen-sounds` writes it out as `beep.wav`. Changing it changes the shipped binary. |
| `warning.wav` | Test fixture. Pins the Rust synthesizer in `crates/alertu-ctl/src/sounds.rs`. |
| `siren.wav` | Test fixture. Same. |
| `logo.png`, `os-alertU.png` | Branding, used by the top-level `README.md`. |

## About the two fixtures

`warning.wav` and `siren.wav` are *reference output*, validated by ear on real
hardware. `sounds::tests::matches_the_committed_reference_files` regenerates
both in Rust and compares them sample by sample (±2 LSB, for the f32/f64 fade
difference), so any accidental change to a synthesis constant fails the test
loudly. They are never installed and never read at runtime.

`docs/superpowers/gensounds.py` is the frozen Python original that produced
them. It is kept for provenance only — the Rust code in `sounds.rs` is the
implementation, and the Python script is not expected to be re-run. If a sound
genuinely needs to change, change `sounds.rs`, listen to the result, and commit
the regenerated fixture alongside it.
