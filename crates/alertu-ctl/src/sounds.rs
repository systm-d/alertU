//! Synthesis of AlertU's default sounds as 16-bit mono PCM WAV.
//!
//! No dependencies: the RIFF header is written by hand and the waveforms are
//! computed directly. Ported from `docs/superpowers/gensounds.py`, whose output
//! was validated on real hardware.

#![allow(dead_code)]

/// Sample rate of every generated file.
pub const SAMPLE_RATE: u32 = 44_100;

/// Siren length, in seconds.
pub const SIREN_SECS: f64 = 2.0;
/// Centre frequency of the siren sweep, in hertz.
pub const SIREN_F_MID: f64 = 1000.0;
/// Peak deviation either side of the centre frequency, in hertz.
pub const SIREN_F_DEV: f64 = 400.0;

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

/// The looping alarm siren: a 600↔1400 Hz sweep that restarts seamlessly.
///
/// Instantaneous frequency is `f_mid + f_dev·sin(2πt/T)`, completing exactly one
/// modulation cycle over the file. Integrating gives the phase below; because
/// `f_mid · T` is a whole number, the phase at the end lands a whole number of
/// cycles from the start, so `siren_loop` respawning the player continues the
/// waveform rather than stepping.
pub fn siren() -> Vec<f32> {
    const AMP: f32 = 0.75;

    let n = (SAMPLE_RATE as f64 * SIREN_SECS) as usize;
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f64 / SAMPLE_RATE as f64;
            let phase = 2.0 * std::f64::consts::PI * SIREN_F_MID * t
                - (SIREN_F_DEV * SIREN_SECS) * (2.0 * std::f64::consts::PI * t / SIREN_SECS).cos();
            AMP * phase.sin() as f32
        })
        .collect();
    fade_ends(&mut out, 8.0);
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

    #[test]
    fn the_siren_is_two_seconds_and_starts_and_ends_silent() {
        let s = siren();
        assert_eq!(s.len(), SAMPLE_RATE as usize * 2);
        assert_eq!(s[0], 0.0);
        assert_eq!(*s.last().unwrap(), 0.0);
        let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.70..=0.80).contains(&peak), "peak was {peak}");
    }

    /// The reason the siren loops without clicking: over one file the phase
    /// advances by a whole number of cycles, so restarting it continues the
    /// waveform. `siren_loop` respawns the player every iteration, so this is a
    /// requirement, not polish.
    #[test]
    fn the_sirens_phase_closes_on_a_whole_number_of_cycles() {
        let advance = SIREN_F_MID * SIREN_SECS;
        assert!(
            (advance - advance.round()).abs() < 1e-9,
            "f_mid * duration must be integral, got {advance}"
        );
    }

    /// The committed reference files came from `docs/superpowers/gensounds.py`.
    /// The Rust port must agree with them on every property that matters; exact
    /// bytes may differ because the fade multiplies in f32 here, f64 there.
    #[test]
    fn matches_the_committed_reference_files() {
        for (generated, reference) in [
            (encode_wav(&warning_tick()), "../../resources/warning.wav"),
            (encode_wav(&siren()), "../../resources/siren.wav"),
        ] {
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
        }
    }
}
