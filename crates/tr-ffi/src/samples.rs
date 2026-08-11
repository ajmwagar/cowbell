//! The sample plane: browsing, classifying and chopping audio on disk.
//!
//! Wraps [`autosample`]'s core analysis API. Same rule as the rest of this
//! crate — no DSP or format knowledge lives here, only the shape Swift wants:
//! owned records, `Result` instead of `anyhow`, and paths as strings.
//!
//! ## Chops are the app's model, not autosample's
//!
//! `autosample::slice::find_slices` is an *onset detector*: it returns marker
//! frames, not regions. A chop needs an in-point and an out-point, and the
//! out-point is where the *next* marker begins — so the region model is derived
//! here, once, rather than in every caller. The last chop runs to the end of
//! the file, which nothing in the marker list can express.
//!
//! ## What is deliberately not surfaced
//!
//! `SlicePoint` carries `suggested_type` and `quality_score`. They are not
//! exposed: `autosample`'s region analyser zeroes several of the features
//! feeding them, so they read as authoritative while being partly unpopulated.
//! Better absent than quietly wrong.

use std::path::{Path, PathBuf};

use autosample::audio;
use autosample::classify::{self, PackProfile};
use autosample::slice::{self, SliceConfig};

use crate::TrError;

fn io_err(path: &Path, e: impl std::fmt::Display) -> TrError {
    TrError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Value types
// ---------------------------------------------------------------------------

/// A sample on disk: what it is and what it sounds like, without loading it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SampleInfo {
    pub path: String,
    /// Filename without the extension — what a browser row shows.
    pub name: String,
    pub duration_secs: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_depth: u16,
    /// Peak amplitude, 0–1. A value of exactly 1.0 means clipped.
    pub peak: f64,
    pub rms: f64,
    /// Integrated loudness (ITU-R BS.1770).
    pub lufs: f64,
    /// Spectral centroid in Hz — the "brightness" a browser sorts by.
    pub centroid_hz: f64,
    /// Peak over RMS in dB. High means punchy; low means squashed or sustained.
    pub crest_db: f64,
    /// `"Kick"`, `"Snare"`, `"Vocal"`… from the classifier.
    pub sample_type: String,
    /// `"OneShot"`, `"Loop"`, `"Phrase"`.
    pub style: String,
    /// `"Dry"` or `"Wet"`.
    pub processing: String,
}

/// One chop: a region between two onsets.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ChopInfo {
    /// In-point, in sample frames.
    pub start_frame: u64,
    /// Out-point, in sample frames — the next onset, or the end of the file.
    pub end_frame: u64,
    pub start_secs: f64,
    pub end_secs: f64,
    /// Onset sharpness at the in-point, 0–1. Useful for filtering weak hits.
    pub onset_strength: f64,
}

impl ChopInfo {
    pub fn frames(&self) -> u64 {
        self.end_frame.saturating_sub(self.start_frame)
    }
}

/// How aggressively to chop.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ChopSettings {
    /// Shortest allowed chop, in seconds.
    pub min_length_secs: f64,
    /// Onset sensitivity, 0–1. Lower finds fewer chops.
    pub sensitivity: f64,
    /// Minimum onset strength to accept, 0–1.
    pub threshold: f64,
    pub max_chops: u32,
}

impl Default for ChopSettings {
    fn default() -> Self {
        let d = SliceConfig::default();
        ChopSettings {
            min_length_secs: d.min_slice_secs,
            sensitivity: d.sensitivity,
            threshold: d.threshold,
            max_chops: d.max_slices as u32,
        }
    }
}

/// A waveform reduced to per-pixel peaks, for drawing.
///
/// Computed here because `autosample` has no drawing API and the app should not
/// pull a multi-megabyte buffer across the FFI just to render a few hundred
/// columns.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WaveformInfo {
    /// Absolute peak per bucket, 0–1, left to right.
    pub peaks: Vec<f32>,
    pub frames: u64,
    pub sample_rate: u32,
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Default chop settings, so the app does not restate autosample's defaults.
#[uniffi::export]
pub fn default_chop_settings() -> ChopSettings {
    ChopSettings::default()
}

/// Every audio file under `dir`, recursively.
#[uniffi::export]
pub fn find_audio_files(dir: String) -> Result<Vec<String>, TrError> {
    let path = PathBuf::from(&dir);
    audio::find_audio_files(&path)
        .map(|v| v.iter().map(|p| p.display().to_string()).collect())
        .map_err(|e| io_err(&path, e))
}

/// Analyse and classify one sample.
///
/// Note `autosample::audio::analyze` reads WAV only; AIFF and FLAC come back as
/// an error rather than silently analysing nothing.
#[uniffi::export]
pub fn describe_sample(path: String) -> Result<SampleInfo, TrError> {
    let p = PathBuf::from(&path);
    let meta = audio::analyze(&p).map_err(|e| io_err(&p, e))?;
    let class = classify::classify_file_with_profile(&p, PackProfile::General)
        .map_err(|e| io_err(&p, e))?;

    Ok(SampleInfo {
        name: p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        path,
        duration_secs: meta.duration_secs,
        sample_rate: meta.sample_rate,
        channels: meta.channels,
        bit_depth: meta.bits_per_sample,
        peak: meta.peak_amplitude,
        rms: meta.rms_energy,
        lufs: meta.lufs,
        centroid_hz: meta.spectral_centroid,
        crest_db: meta.crest_factor_db,
        sample_type: format!("{:?}", class.sample_type),
        style: format!("{:?}", class.style),
        processing: format!("{:?}", class.processing),
    })
}

/// Chop a file at its transients.
///
/// Returns regions, not markers: each chop ends where the next begins, and the
/// last runs to the end of the file.
#[uniffi::export]
pub fn chop_sample(path: String, settings: ChopSettings) -> Result<Vec<ChopInfo>, TrError> {
    let p = PathBuf::from(&path);
    let (samples, rate) = audio::read_wav_mono(&p).map_err(|e| io_err(&p, e))?;

    let config = SliceConfig {
        min_slice_secs: settings.min_length_secs,
        sensitivity: settings.sensitivity,
        threshold: settings.threshold,
        max_slices: settings.max_chops as usize,
        profile: PackProfile::General,
    };

    let points = slice::find_slices(&samples, rate, &config);
    let total = samples.len() as u64;

    Ok(points
        .iter()
        .enumerate()
        .map(|(i, sp)| {
            // The out-point is the next onset; the last chop runs to the end.
            let end = points
                .get(i + 1)
                .map(|next| next.frame as u64)
                .unwrap_or(total);
            ChopInfo {
                start_frame: sp.frame as u64,
                end_frame: end,
                start_secs: sp.time_secs,
                end_secs: end as f64 / rate.max(1) as f64,
                onset_strength: sp.onset_strength,
            }
        })
        // A zero-length chop is a detector artefact, never a thing to play.
        .filter(|c| c.frames() > 0)
        .collect())
}

/// Reduce a file to `buckets` peak values for drawing.
#[uniffi::export]
pub fn waveform(path: String, buckets: u32) -> Result<WaveformInfo, TrError> {
    let p = PathBuf::from(&path);
    let (samples, rate) = audio::read_wav_mono(&p).map_err(|e| io_err(&p, e))?;

    let buckets = (buckets.max(1) as usize).min(samples.len().max(1));
    let per = (samples.len() as f64 / buckets as f64).max(1.0);

    let peaks = (0..buckets)
        .map(|i| {
            let start = (i as f64 * per) as usize;
            let end = (((i + 1) as f64 * per) as usize).min(samples.len());
            samples[start..end.max(start)]
                .iter()
                .fold(0f32, |acc, &s| acc.max(s.abs() as f32))
        })
        .collect();

    Ok(WaveformInfo {
        peaks,
        frames: samples.len() as u64,
        sample_rate: rate,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A WAV with `hits` sharp transients, written to a temp path.
    fn transient_wav(hits: usize, rate: u32) -> PathBuf {
        let path = std::env::temp_dir().join(format!("trffi-chop-{hits}-{rate}.wav"));
        let per = rate as usize / 2; // half a second between hits
        let total = per * hits;

        let mut data = Vec::with_capacity(total * 2);
        for i in 0..total {
            let into_hit = i % per;
            // A decaying click at the start of each region.
            let amp = if into_hit < rate as usize / 50 {
                1.0 - (into_hit as f64 / (rate as f64 / 50.0))
            } else {
                0.0
            };
            let v = (amp * 30000.0) as i16;
            data.extend_from_slice(&v.to_le_bytes());
        }

        let mut f = std::fs::File::create(&path).unwrap();
        let data_len = data.len() as u32;
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
        f.write_all(b"WAVEfmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
        f.write_all(&1u16.to_le_bytes()).unwrap(); // mono
        f.write_all(&rate.to_le_bytes()).unwrap();
        f.write_all(&(rate * 2).to_le_bytes()).unwrap();
        f.write_all(&2u16.to_le_bytes()).unwrap();
        f.write_all(&16u16.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&data_len.to_le_bytes()).unwrap();
        f.write_all(&data).unwrap();
        path
    }

    #[test]
    fn describes_a_sample() {
        let path = transient_wav(4, 44100);
        let info = describe_sample(path.display().to_string()).unwrap();

        assert_eq!(info.sample_rate, 44100);
        assert_eq!(info.channels, 1);
        assert!(info.duration_secs > 1.9 && info.duration_secs < 2.1);
        assert!(info.peak > 0.5, "the clicks should register as peaks");
        assert!(!info.sample_type.is_empty());
        let _ = std::fs::remove_file(path);
    }

    /// The property that makes chops usable: they are regions that tile the
    /// file, not bare markers.
    #[test]
    fn chops_are_contiguous_regions_ending_at_the_file_end() {
        let path = transient_wav(4, 44100);
        let chops = chop_sample(path.display().to_string(), ChopSettings::default()).unwrap();

        assert!(!chops.is_empty(), "a file of clicks should chop");
        for pair in chops.windows(2) {
            assert_eq!(
                pair[0].end_frame, pair[1].start_frame,
                "each chop must end where the next begins"
            );
        }
        assert!(chops.iter().all(|c| c.frames() > 0), "no empty chops");

        let last = chops.last().unwrap();
        assert_eq!(
            last.end_frame, 44100 * 2,
            "the last chop runs to the end of the file"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_stricter_threshold_finds_no_more_chops() {
        let path = transient_wav(6, 44100);
        let loose = chop_sample(path.display().to_string(), ChopSettings::default()).unwrap();

        let strict = chop_sample(
            path.display().to_string(),
            ChopSettings {
                threshold: 0.95,
                ..ChopSettings::default()
            },
        )
        .unwrap();

        assert!(strict.len() <= loose.len());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn waveform_reduces_to_the_requested_buckets() {
        let path = transient_wav(4, 44100);
        let w = waveform(path.display().to_string(), 200).unwrap();

        assert_eq!(w.peaks.len(), 200);
        assert_eq!(w.frames, 44100 * 2);
        assert!(w.peaks.iter().all(|&p| (0.0..=1.0).contains(&p)));
        assert!(w.peaks.iter().any(|&p| p > 0.5), "clicks should show");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_missing_file_is_an_io_error_not_a_panic() {
        let err = describe_sample("/nonexistent/nope.wav".into()).unwrap_err();
        assert!(matches!(err, TrError::Io { .. }), "got {err:?}");
        assert!(chop_sample("/nonexistent/nope.wav".into(), ChopSettings::default()).is_err());
    }
}
