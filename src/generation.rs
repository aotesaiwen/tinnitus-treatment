//! Reproducible music sources, kept separate from clinical processing.
//! Neither prompts nor synthesis parameters select a trial arm or a notch.

#[cfg(feature = "generation-http")]
pub mod acestep;

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};

use crate::signals::Rng;
use crate::wav::Audio;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Composer,
    AceStep,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    Drift,
    Focus,
    Unwind,
}

impl Style {
    pub fn title(self) -> &'static str {
        match self {
            Self::Drift => "Slow drift",
            Self::Focus => "Gentle focus",
            Self::Unwind => "Evening unwind",
        }
    }

    pub fn prompt(self) -> &'static str {
        match self {
            Self::Drift => {
                "Ambient instrumental, warm sustained pads, spacious slow harmony, delicate soft bells"
            }
            Self::Focus => {
                "Minimal instrumental, gentle electric piano arpeggios, steady unobtrusive pulse, soft warm pads"
            }
            Self::Unwind => {
                "Quiet instrumental, mellow plucked strings, long soft chords, sparse descending melody"
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub style: Style,
    pub duration_s: u32,
    pub tempo_bpm: u32,
    pub seed: u32,
    #[serde(default)]
    pub prompt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recipe {
    pub provider: Provider,
    pub request: Request,
}

impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if !(30..=300).contains(&self.duration_s) {
            return Err("Choose a duration between 30 and 300 seconds.".into());
        }
        if !(45..=100).contains(&self.tempo_bpm) {
            return Err("Choose a tempo between 45 and 100 BPM.".into());
        }
        if self.prompt.chars().count() > 500 {
            return Err("Keep the music description under 500 characters.".into());
        }
        Ok(())
    }

    pub fn caption(&self) -> String {
        format!(
            "{}. {}. Instrumental only, no vocals, gentle dynamics, no sudden loud hits, gradual opening and ending.",
            self.style.prompt(),
            self.prompt.trim()
        )
    }
}

/// A small additive composer, not a neural model. Seed controls key, note
/// selection and stereo texture. Render at a fixed source rate; the player
/// resamples and verifies at the device rate before playback.
pub fn compose(request: &Request, cancelled: impl Fn() -> bool) -> Result<Audio, String> {
    request.validate()?;
    let fs = 48_000;
    let n = request.duration_s as usize * fs as usize;
    let mut channels = vec![vec![0.0f32; n], vec![0.0f32; n]];
    let (left, right) = channels.split_at_mut(1);
    let mut rng = Rng::new(request.seed as u64 + 1);
    let root = 48.0 + (request.seed % 5) as f64;
    let beat = 60.0 / request.tempo_bpm as f64;
    let chord_len = beat * 8.0;
    let progression = [0.0, 5.0, 9.0, 7.0];
    let (spacing, decay, pad_gain) = match request.style {
        Style::Drift => (beat * 4.0, 3.5, 0.15),
        Style::Focus => (beat, 1.2, 0.09),
        Style::Unwind => (beat * 2.0, 2.5, 0.13),
    };
    let hz = |midi: f64| 440.0 * 2f64.powf((midi - 69.0) / 12.0);
    // Overlapping chord envelopes and phase-continuous voices avoid clicks.
    for chord in 0..(request.duration_s as f64 / chord_len).ceil() as usize {
        if cancelled() {
            return Err("Generation cancelled.".into());
        }
        let start = chord as f64 * chord_len;
        let notes = [0.0, 4.0, 7.0, 11.0];
        for (voice, interval) in notes.iter().enumerate() {
            let freq = hz(root + progression[chord % 4] + interval - 12.0);
            let pan = 0.2 + voice as f64 * 0.2;
            for (i, (l, r)) in left[0]
                .iter_mut()
                .zip(&mut right[0])
                .enumerate()
                .take(n.min(((start + chord_len + 2.0) * fs as f64) as usize))
                .skip((start * fs as f64) as usize)
            {
                let t = i as f64 / fs as f64 - start;
                let attack = (t / 2.0).min(1.0);
                let release = ((chord_len + 2.0 - t) / 2.0).clamp(0.0, 1.0);
                let env = (attack * release).powi(2);
                let phase = TAU * freq * t;
                let sample = pad_gain * env / 4.0
                    * (phase.sin() + 0.22 * (2.0 * phase).sin() + 0.08 * (3.0 * phase).sin());
                *l += (sample * (1.0 - pan).sqrt()) as f32;
                *r += (sample * pan.sqrt()) as f32;
            }
        }
    }
    let scale = [0.0, 4.0, 7.0, 12.0, 14.0, 19.0];
    for note in 0..(request.duration_s as f64 / spacing) as usize {
        if cancelled() {
            return Err("Generation cancelled.".into());
        }
        let start = note as f64 * spacing;
        let pick = ((rng.uniform() * scale.len() as f64) as usize).min(scale.len() - 1);
        let chord = (start / chord_len) as usize;
        let freq = hz(root + progression[chord % 4] + scale[pick]);
        let pan = 0.25 + 0.5 * rng.uniform();
        for (i, (l, r)) in left[0]
            .iter_mut()
            .zip(&mut right[0])
            .enumerate()
            .take(n.min(((start + decay * 5.0) * fs as f64) as usize))
            .skip((start * fs as f64) as usize)
        {
            let t = i as f64 / fs as f64 - start;
            let env = (t / 0.035).min(1.0).powi(2) * (-t / decay).exp();
            let phase = TAU * freq * t;
            let sample =
                0.085 * env * (phase.sin() + 0.3 * (phase * 2.0).sin() + 0.1 * (phase * 3.0).sin());
            *l += (sample * (1.0 - pan).sqrt()) as f32;
            *r += (sample * pan.sqrt()) as f32;
        }
    }
    // Quiet, broadband air is part of the composition, not a claim that the
    // source has clinically useful energy in any particular band.
    for i in 0..n {
        if i % 48_000 == 0 && cancelled() {
            return Err("Generation cancelled.".into());
        }
        let t = i as f64 / fs as f64;
        let fade = (t / 2.0).min(1.0) * ((n - 1 - i) as f64 / fs as f64 / 3.0).min(1.0);
        for channel in &mut channels {
            channel[i] = ((channel[i] as f64 + 0.003 * (rng.uniform() * 2.0 - 1.0)) * fade) as f32;
        }
    }
    Ok(Audio { fs, channels })
}
