//! On-disk state: profile, settings, listening log, saved reports.
//! Everything stays on this computer.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tmdsp::generation::{Provider, Recipe, Request, Style};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MusicPreferences {
    pub provider: Provider,
    pub style: Style,
    pub duration_s: u32,
    pub tempo_bpm: u32,
    pub prompt: String,
}

impl Default for MusicPreferences {
    fn default() -> Self {
        Self {
            provider: Provider::Composer,
            style: Style::Drift,
            duration_s: 60,
            tempo_bpm: 54,
            prompt: String::new(),
        }
    }
}

impl MusicPreferences {
    pub fn validate(&self) -> Result<(), String> {
        Request {
            style: self.style,
            duration_s: self.duration_s,
            tempo_bpm: self.tempo_bpm,
            seed: 0,
            prompt: self.prompt.clone(),
        }
        .validate()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryPreferences {
    pub music: MusicPreferences,
    pub favorites: BTreeSet<String>,
}

pub type ListenerPreferences = BTreeMap<String, LibraryPreferences>;

/// Publish complete bytes with a same-directory rename. Failure preserves the
/// previous destination; a unique pending file avoids colliding with other saves.
pub fn atomic_write(destination: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let name = destination
        .file_name()
        .ok_or("Choose a file name.")?
        .to_string_lossy();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let pending = destination.with_file_name(format!(
        ".{name}.{}-{stamp}-{}.pending",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|e| format!("Could not create the output file: {e}"))?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    let result = written.and_then(|_| fs::rename(&pending, destination));
    if result.is_err() {
        let _ = fs::remove_file(pending);
    }
    result.map_err(|e| format!("Could not save the file: {e}"))
}

/// Identity is independent of library sort order and display title.
pub fn track_id(path: &Path) -> String {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    format!(
        "{:x}",
        Sha256::digest(resolved.as_os_str().as_encoded_bytes())
    )
}

/// Return only creation inputs; clinical settings remain in the private record.
pub fn read_recipe(path: &Path) -> Option<Recipe> {
    let file = fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(1024 * 1024).read_to_string(&mut text).ok()?;
    let recipe: Recipe = serde_json::from_str(&text).ok()?;
    recipe.request.validate().ok()?;
    Some(recipe)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arm {
    Notch,
    Passthrough,
    Sham,
}

/// Set up by the clinician. The participant UI never shows `arm`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub participant_id: String,
    pub arm: Arm,
    pub pitch_hz: Option<f64>,
    pub sham_hz: Option<f64>,
    pub width_oct: f64,
    pub daily_goal_min: u32,
    pub matching_report: Option<String>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    pub library_dir: Option<String>,
    pub pin_sha256: Option<String>,
    pub volume_db: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub participant_id: String,
    pub track: String,
    pub started_at: String,
    pub ended_at: String,
    pub seconds: f64,
    pub volume_db: f32,
    pub report: String,
}

pub struct Store {
    pub dir: PathBuf,
}

pub fn hash_pin(pin: &str) -> String {
    Sha256::digest(format!("tinnitus-music:{pin}").as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Store {
    pub fn open(dir: PathBuf) -> std::io::Result<Self> {
        for sub in ["reports", "matching", "library"] {
            fs::create_dir_all(dir.join(sub))?;
        }
        Ok(Self { dir })
    }

    fn read<T: for<'de> Deserialize<'de>>(&self, name: &str) -> Option<T> {
        serde_json::from_str(&fs::read_to_string(self.dir.join(name)).ok()?).ok()
    }

    fn write<T: Serialize>(&self, name: &str, value: &T) -> Result<(), String> {
        let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
        atomic_write(&self.dir.join(name), text.as_bytes())
            .map_err(|e| format!("saving {name}: {e}"))
    }

    pub fn profile(&self) -> Option<Profile> {
        self.read("profile.json")
    }

    pub fn save_profile(&self, p: &Profile) -> Result<(), String> {
        self.write("profile.json", p)
    }

    pub fn settings(&self) -> Settings {
        self.read("settings.json").unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) -> Result<(), String> {
        self.write("settings.json", s)
    }

    pub fn listener_preferences(&self) -> ListenerPreferences {
        self.read("library_preferences.json").unwrap_or_default()
    }

    pub fn save_listener_preferences(
        &self,
        preferences: &ListenerPreferences,
    ) -> Result<(), String> {
        self.write("library_preferences.json", preferences)
    }

    /// Writes `value` to `<sub>/<name>.json`; returns the file name.
    pub fn save_record<T: Serialize>(
        &self,
        sub: &str,
        name: &str,
        value: &T,
    ) -> Result<String, String> {
        let file = format!("{name}.json");
        self.write(&format!("{sub}/{file}"), value)?;
        Ok(file)
    }

    pub fn append_log(&self, e: &LogEntry) -> Result<(), String> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("listening_log.jsonl"))
            .map_err(|e| format!("opening listening log: {e}"))?;
        let line = serde_json::to_string(e).map_err(|e| e.to_string())?;
        writeln!(f, "{line}").map_err(|e| format!("writing listening log: {e}"))
    }

    /// Keep history isolated when a computer is reassigned to another listener.
    pub fn log_for(&self, participant_id: &str) -> Vec<LogEntry> {
        let Ok(f) = fs::File::open(self.dir.join("listening_log.jsonl")) else {
            return Vec::new();
        };
        BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str::<LogEntry>(&l).ok())
            .filter(|entry| entry.participant_id == participant_id)
            .collect()
    }

    pub fn library_dir(&self, settings: &Settings) -> PathBuf {
        settings
            .library_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.dir.join("library"))
    }
}

/// Duration of a WAV file from its header, without reading the samples.
pub fn wav_duration(path: &Path) -> Option<f64> {
    let mut f = fs::File::open(path).ok()?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return None;
    }
    let (mut byte_rate, mut data_len) = (0u32, None);
    loop {
        let mut ch = [0u8; 8];
        if f.read_exact(&mut ch).is_err() {
            break;
        }
        let size = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]);
        match &ch[0..4] {
            b"fmt " => {
                let mut fmt = [0u8; 16];
                f.read_exact(&mut fmt).ok()?;
                byte_rate = u32::from_le_bytes([fmt[8], fmt[9], fmt[10], fmt[11]]);
                f.seek(SeekFrom::Current(size as i64 - 16 + (size & 1) as i64))
                    .ok()?;
            }
            b"data" => {
                data_len = Some(size);
                break;
            }
            _ => {
                f.seek(SeekFrom::Current(size as i64 + (size & 1) as i64))
                    .ok()?;
            }
        }
    }
    (byte_rate > 0).then(|| data_len.unwrap_or(0) as f64 / byte_rate as f64)
}

/// "t03_acoustic_guitar" → "Acoustic guitar". Titles never mention processing.
pub fn track_title(stem: &str) -> String {
    let rest = match stem.split_once('_') {
        Some((head, tail))
            if head.len() <= 4 && head.chars().skip(1).all(|c| c.is_ascii_digit()) =>
        {
            tail
        }
        _ => stem,
    };
    let words = rest.replace(['_', '-'], " ");
    let mut c = words.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => stem.to_string(),
    }
}
