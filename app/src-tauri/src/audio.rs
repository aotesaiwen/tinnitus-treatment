//! Audio output on a dedicated thread (cpal). Commands swap in a whole
//! interleaved stereo buffer; the callback plays it with a smoothed gain.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

#[derive(Default)]
struct Shared {
    /// Interleaved stereo at `fs`.
    buffer: Arc<Vec<f32>>,
    pos: usize,
    playing: bool,
    gain: f32,
    gain_target: f32,
    /// Frames output while playing, ever. Callers take differences.
    played_frames: u64,
    finished: bool,
    fs: u32,
    ready: bool,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub finished: bool,
    pub pos_frames: usize,
    pub played_frames: u64,
    pub fs: u32,
}

enum Msg {
    Rebuild,
}

pub struct AudioEngine {
    shared: Arc<Mutex<Shared>>,
    tx: Mutex<mpsc::Sender<Msg>>,
}

impl AudioEngine {
    pub fn start() -> Arc<Self> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (tx, rx) = mpsc::channel();
        let s = shared.clone();
        std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || audio_thread(s, rx))
            .expect("spawning audio thread");
        let engine = Arc::new(Self { shared, tx: Mutex::new(tx) });
        engine.wait_ready();
        engine
    }

    fn wait_ready(&self) {
        for _ in 0..100 {
            if self.shared.lock().unwrap().ready {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Device sample rate; rebuilds the stream first if the device failed
    /// (e.g. headphones unplugged).
    pub fn device_fs(&self) -> Result<u32, String> {
        let failed = self.shared.lock().unwrap().error.is_some();
        if failed {
            {
                let mut s = self.shared.lock().unwrap();
                s.ready = false;
                s.error = None;
            }
            let _ = self.tx.lock().unwrap().send(Msg::Rebuild);
            self.wait_ready();
        }
        let s = self.shared.lock().unwrap();
        match (&s.error, s.fs) {
            (Some(e), _) => Err(e.clone()),
            (None, 0) => Err("no audio output device".into()),
            (None, fs) => Ok(fs),
        }
    }

    /// Starts playing `stereo`. `fade_in` ramps the gain up over ~10 ms;
    /// test stimuli carry their own ramps and start at full gain.
    pub fn load(&self, stereo: Arc<Vec<f32>>, gain: f32, fade_in: bool) {
        let mut s = self.shared.lock().unwrap();
        s.buffer = stereo;
        s.pos = 0;
        s.finished = false;
        s.gain_target = gain;
        s.gain = if fade_in { 0.0 } else { gain };
        s.playing = true;
    }

    pub fn set_playing(&self, playing: bool) {
        let mut s = self.shared.lock().unwrap();
        if !s.finished {
            s.playing = playing;
        }
    }

    pub fn set_gain(&self, gain: f32) {
        self.shared.lock().unwrap().gain_target = gain;
    }

    pub fn stop(&self) {
        let mut s = self.shared.lock().unwrap();
        s.playing = false;
        s.gain = 0.0;
        s.buffer = Arc::new(Vec::new());
        s.pos = 0;
    }

    pub fn snapshot(&self) -> Snapshot {
        let s = self.shared.lock().unwrap();
        Snapshot {
            finished: s.finished,
            pos_frames: s.pos,
            played_frames: s.played_frames,
            fs: s.fs,
        }
    }

    pub fn error(&self) -> Option<String> {
        self.shared.lock().unwrap().error.clone()
    }
}

fn audio_thread(shared: Arc<Mutex<Shared>>, rx: mpsc::Receiver<Msg>) {
    loop {
        let stream = match build_stream(&shared) {
            Ok(stream) => Some(stream),
            Err(e) => {
                shared.lock().unwrap().error = Some(e);
                None
            }
        };
        shared.lock().unwrap().ready = true;
        match rx.recv() {
            Ok(Msg::Rebuild) => drop(stream),
            Err(_) => return,
        }
    }
}

fn build_stream(shared: &Arc<Mutex<Shared>>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device")?;
    let config: cpal::StreamConfig = device.default_output_config().map_err(|e| e.to_string())?.into();
    let channels = config.channels as usize;
    let fs = config.sample_rate as u32;
    {
        let mut s = shared.lock().unwrap();
        s.fs = fs;
        s.error = None;
    }
    let coef = 1.0 - (-1.0 / (0.01 * fs as f32)).exp();
    let sh = shared.clone();
    let err_shared = shared.clone();
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Never block the audio thread: on contention, output silence.
                let Ok(mut guard) = sh.try_lock() else {
                    data.fill(0.0);
                    return;
                };
                let s = &mut *guard;
                let len = s.buffer.len() / 2;
                for frame in data.chunks_mut(channels) {
                    let target = if s.playing { s.gain_target } else { 0.0 };
                    s.gain += (target - s.gain) * coef;
                    let (l, r) = if s.pos < len && (s.playing || s.gain > 1e-4) {
                        let i = 2 * s.pos;
                        s.pos += 1;
                        (s.buffer[i] * s.gain, s.buffer[i + 1] * s.gain)
                    } else {
                        (0.0, 0.0)
                    };
                    if s.playing {
                        s.played_frames += 1;
                        if s.pos >= len {
                            s.playing = false;
                            s.finished = true;
                        }
                    }
                    if channels == 1 {
                        frame[0] = 0.5 * (l + r);
                    } else {
                        frame[0] = l;
                        frame[1] = r;
                        frame[2..].fill(0.0);
                    }
                }
            },
            move |err: cpal::Error| {
                if !matches!(err.kind(), cpal::ErrorKind::Xrun) {
                    err_shared.lock().unwrap().error = Some(format!("audio device: {err}"));
                }
            },
            None,
        )
        .map_err(|e| format!("opening audio output: {e}"))?;
    stream.play().map_err(|e| format!("starting audio output: {e}"))?;
    Ok(stream)
}
