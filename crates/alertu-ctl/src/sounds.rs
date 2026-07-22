//! Synthesis of the countdown tick, as a 16-bit mono PCM WAV.
//!
//! No dependencies: the RIFF header is written by hand and the waveform is
//! computed directly. Ported from `docs/superpowers/gensounds.py`, whose output
//! is committed as `assets/warning.wav` and pins this code.
//!
//! Only the tick is synthesized. The chirp and the siren are recordings
//! embedded by `main.rs` — they sound better than a sine sweep. The tick stays
//! generated because it must be a few tens of milliseconds: `play_once` spawns a
//! player once a second during the countdown, so a longer file would pile up.

/// Sample rate of the generated file.
pub const SAMPLE_RATE: u32 = 44_100;

/// Encode mono samples in `[-1.0, 1.0]` as a 16-bit PCM WAV file.
///
/// Values outside the range are clamped rather than allowed to wrap, which
/// would turn a loud peak into a burst of noise.
pub fn encode_wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // format: uncompressed PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // channels: mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Fade both ends to silence over `ms` milliseconds, in place.
///
/// Every generated file gets this: a waveform that starts or ends on a non-zero
/// sample produces an audible click when a player opens or closes it.
fn fade_ends(samples: &mut [f32], ms: f64) {
    let n = (SAMPLE_RATE as f64 * ms / 1000.0) as usize;
    let n = n.min(samples.len() / 2);
    if n == 0 {
        return;
    }
    let last = samples.len() - 1;
    for i in 0..n {
        let gain = i as f32 / n as f32;
        samples[i] *= gain;
        samples[last - i] *= gain;
    }
}

/// The discreet countdown tick played once a second while `Triggered`.
pub fn warning_tick() -> Vec<f32> {
    const SECS: f64 = 0.05;
    const FREQ: f64 = 1200.0;
    const AMP: f32 = 0.18;

    let n = (SAMPLE_RATE as f64 * SECS) as usize;
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f64 / SAMPLE_RATE as f64;
            AMP * (2.0 * std::f64::consts::PI * FREQ * t).sin() as f32
        })
        .collect();
    fade_ends(&mut out, 6.0);
    out
}

#[cfg(test)]
mod tests {
    use crate::sounds::*;

    /// Read a little-endian u32 at `off`.
    fn u32_at(bytes: &[u8], off: usize) -> u32 {
        u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
    }

    /// Read a little-endian u16 at `off`.
    fn u16_at(bytes: &[u8], off: usize) -> u16 {
        u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap())
    }

    /// Decode the samples back out of an encoded WAV.
    fn samples_of(wav: &[u8]) -> Vec<i16> {
        wav[44..]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    #[test]
    fn header_is_a_valid_mono_16bit_riff() {
        let wav = encode_wav(&[0.0; 100]);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");

        assert_eq!(u32_at(&wav, 16), 16, "PCM fmt chunk is 16 bytes");
        assert_eq!(u16_at(&wav, 20), 1, "format 1 = uncompressed PCM");
        assert_eq!(u16_at(&wav, 22), 1, "mono");
        assert_eq!(u32_at(&wav, 24), SAMPLE_RATE);
        assert_eq!(u32_at(&wav, 28), SAMPLE_RATE * 2, "byte rate");
        assert_eq!(u16_at(&wav, 32), 2, "block align");
        assert_eq!(u16_at(&wav, 34), 16, "bits per sample");
    }

    #[test]
    fn chunk_sizes_match_the_payload() {
        let wav = encode_wav(&[0.0; 100]);
        assert_eq!(wav.len(), 44 + 200);
        assert_eq!(u32_at(&wav, 40), 200, "data chunk size");
        assert_eq!(u32_at(&wav, 4), 36 + 200, "RIFF chunk size");
    }

    #[test]
    fn samples_are_scaled_and_clamped() {
        let wav = encode_wav(&[0.0, 1.0, -1.0, 2.0, -2.0]);
        assert_eq!(samples_of(&wav), vec![0, 32767, -32767, 32767, -32767]);
    }

    #[test]
    fn the_tick_is_short_quiet_and_starts_and_ends_silent() {
        let s = warning_tick();
        // f64, matching the implementation: 44100_f32 * 0.05_f32 can land just
        // under 2205.0 and truncate to 2204.
        assert_eq!(s.len(), (SAMPLE_RATE as f64 * 0.05) as usize);
        assert_eq!(s[0], 0.0);
        assert_eq!(*s.last().unwrap(), 0.0);
        let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.15..=0.20).contains(&peak), "peak was {peak}");
    }

    /// The committed reference came from `docs/superpowers/gensounds.py` and was
    /// validated by ear. Its path is anchored to `CARGO_MANIFEST_DIR` rather than
    /// the process working directory, matching the `include_bytes!` convention
    /// next door and surviving any runner that does not start the test binary in
    /// the package root.
    ///
    /// The Rust port must agree with it on every property that matters; exact
    /// bytes may differ because the fade multiplies in f32 here, f64 there — up
    /// to ±2 LSB per sample is tolerated for that reason, but no more, so a
    /// genuinely wrong formula still fails.
    #[test]
    fn matches_the_committed_reference_file() {
        for (generated, reference) in [(
            encode_wav(&warning_tick()),
            concat!(env!("CARGO_MANIFEST_DIR"), "/assets/warning.wav"),
        )] {
            let expected =
                std::fs::read(reference).unwrap_or_else(|e| panic!("reading {reference}: {e}"));
            assert_eq!(
                generated.len(),
                expected.len(),
                "{reference}: length differs"
            );
            assert_eq!(
                &generated[0..44],
                &expected[0..44],
                "{reference}: header differs"
            );

            let got = samples_of(&generated);
            let want = samples_of(&expected);
            for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
                let diff = (i32::from(*g) - i32::from(*w)).abs();
                assert!(
                    diff <= 2,
                    "{reference}: sample {i} differs by more than 2 LSB: got {g}, want {w}"
                );
            }
        }
    }
}
