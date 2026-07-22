# resources/

Branding for the repository, not an install directory.

| File | Role |
|------|------|
| `logo.png` | The application icon master, 1024×1024 with a transparent background. `packaging/icons/` holds the sizes derived from it. |
| `os-alertU.png` | The banner at the top of the top-level `README.md`. |

## Where the sounds went

They live in `crates/alertu-ctl/assets/`, because `cargo package` only includes
files inside the crate it is packaging — keeping them here would make
`alertu-ctl` unpublishable. See that directory's role in
`crates/alertu-ctl/src/main.rs` and `sounds.rs`.

**The sounds users install come from `alertu-ctl gen-sounds`, not from copying
files.** The documented step is:

```sh
sudo alertu-ctl gen-sounds --dir /usr/share/sounds/alertu
```

which writes `beep.wav`, `warning.wav` and `siren.wav` with the right modes
(0644, in a 0755 directory) so the unprivileged `alertu` service account can
read them. Copying files by hand skips that and produces sounds the daemon may
silently fail to play.
