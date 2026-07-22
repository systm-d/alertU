# assets/

Sound files belonging to `alertu-ctl`. They live inside the crate rather than in
the repository's `resources/` because `cargo package` only includes files under
the crate being packaged — reaching outside it would make this crate
unpublishable.

| File | Role |
|------|------|
| `lock.wav` | The arm/disarm chirp. **Embedded into the binary** by `include_bytes!` in `src/main.rs`; `gen-sounds` writes it out as `beep.wav`. Changing it changes the shipped binary. |
| `siren.wav` | The alarm siren. Also embedded, written out as `siren.wav`. A recording rather than synthesis — it sounds better than a sweep. It starts and ends at silence, which matters because `siren_loop` respawns the player on every iteration. |
| `warning.wav` | **Test fixture only**, never embedded. The countdown tick is generated at runtime by `sounds::warning_tick()`; this file is the reference output that pins it. |

## Why the tick is the one that stays synthesized

`play_once` spawns a player once a second during the countdown, so the tick has
to be a few tens of milliseconds — anything longer piles up into overlapping
copies. Fifty milliseconds of sine with a short fade is exactly the job, and
generating it costs nothing in binary size.

`sounds::tests::matches_the_committed_reference_file` regenerates the tick and
compares it to `warning.wav` sample by sample (±2 LSB, for the f32/f64 fade
difference), so an accidental change to a synthesis constant fails loudly.
`docs/superpowers/gensounds.py` is the frozen Python original that produced it,
kept for provenance and not expected to be re-run.
