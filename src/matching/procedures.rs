//! Adaptive psychophysical procedures as pure state machines: each one says
//! what to present next and updates on the answer.

use serde::Serialize;

/// Outcome of a level procedure. `AboveMax`/`BelowMin` carry the limit that
/// was reached, e.g. tinnitus louder than the loudest allowed tone.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "status", content = "db", rename_all = "snake_case")]
pub enum Level {
    Found(f64),
    AboveMax(f64),
    BelowMin(f64),
}

impl Level {
    pub fn db(&self) -> f64 {
        match *self {
            Level::Found(v) | Level::AboveMax(v) | Level::BelowMin(v) => v,
        }
    }

    pub fn found(&self) -> Option<f64> {
        match *self {
            Level::Found(v) => Some(v),
            _ => None,
        }
    }
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// Modified Hughson-Westlake: down 10 dB after "heard", up 5 dB after
/// "not heard"; threshold is the first level heard on two ascending runs.
#[derive(Clone, Debug)]
pub struct AscendingThreshold {
    level: f64,
    min: f64,
    max: f64,
    prev_no: bool,
    ascending_yes: Vec<f64>,
    trials: usize,
    pub result: Option<Level>,
}

impl AscendingThreshold {
    const MAX_TRIALS: usize = 30;

    pub fn new(start: f64, min: f64, max: f64) -> Self {
        Self { level: start.clamp(min, max), min, max, prev_no: false, ascending_yes: Vec::new(), trials: 0, result: None }
    }

    pub fn level(&self) -> f64 {
        self.level
    }

    pub fn respond(&mut self, heard: bool) {
        if self.result.is_some() {
            return;
        }
        self.trials += 1;
        if heard {
            if self.prev_no {
                self.ascending_yes.push(self.level);
                let hits = self.ascending_yes.iter().filter(|&&l| (l - self.level).abs() < 0.01).count();
                if hits >= 2 {
                    self.result = Some(Level::Found(self.level));
                    return;
                }
            }
            if self.level <= self.min {
                self.result = Some(Level::BelowMin(self.min));
                return;
            }
            self.prev_no = false;
            self.level = (self.level - 10.0).max(self.min);
        } else {
            if self.level >= self.max {
                self.result = Some(Level::AboveMax(self.max));
                return;
            }
            self.prev_no = true;
            self.level = (self.level + 5.0).min(self.max);
        }
        if self.trials >= Self::MAX_TRIALS {
            let lowest = self.ascending_yes.iter().copied().fold(f64::INFINITY, f64::min);
            self.result = Some(if lowest.is_finite() { Level::Found(lowest) } else { Level::AboveMax(self.max) });
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoudnessJudgement {
    /// The sound is louder than the tinnitus.
    Louder,
    Softer,
    Equal,
}

/// Loudness match by staircase: 5, 2, then 1 dB steps, shrinking at each
/// reversal. Ends after two "equal" answers or four reversals at 1 dB.
#[derive(Clone, Debug)]
pub struct LoudnessMatch {
    level: f64,
    min: f64,
    max: f64,
    step: f64,
    last_dir: i8,
    fine_reversals: Vec<f64>,
    equals: Vec<f64>,
    trials: usize,
    pub result: Option<Level>,
}

impl LoudnessMatch {
    const MAX_TRIALS: usize = 30;

    pub fn new(start: f64, min: f64, max: f64) -> Self {
        Self {
            level: start.clamp(min, max),
            min,
            max,
            step: 5.0,
            last_dir: 0,
            fine_reversals: Vec::new(),
            equals: Vec::new(),
            trials: 0,
            result: None,
        }
    }

    pub fn level(&self) -> f64 {
        self.level
    }

    pub fn respond(&mut self, j: LoudnessJudgement) {
        if self.result.is_some() {
            return;
        }
        self.trials += 1;
        let dir: i8 = match j {
            LoudnessJudgement::Softer => {
                if self.level >= self.max {
                    self.result = Some(Level::AboveMax(self.max));
                    return;
                }
                1
            }
            LoudnessJudgement::Louder => {
                if self.level <= self.min {
                    self.result = Some(Level::BelowMin(self.min));
                    return;
                }
                -1
            }
            LoudnessJudgement::Equal => {
                self.equals.push(self.level);
                if self.equals.len() >= 2 {
                    self.result = Some(Level::Found(mean(&self.equals)));
                    return;
                }
                // Probe the other side to confirm.
                if self.last_dir == 0 { 1 } else { -self.last_dir }
            }
        };
        if self.last_dir != 0 && dir != self.last_dir {
            if self.step <= 1.0 {
                self.fine_reversals.push(self.level);
                if self.fine_reversals.len() >= 4 {
                    self.result = Some(Level::Found(mean(&self.fine_reversals)));
                    return;
                }
            }
            self.step = if self.step > 2.0 { 2.0 } else { 1.0 };
        }
        self.last_dir = dir;
        self.level = (self.level + dir as f64 * self.step).clamp(self.min, self.max);
        if self.trials >= Self::MAX_TRIALS {
            let est = if !self.fine_reversals.is_empty() { mean(&self.fine_reversals) } else { self.level };
            self.result = Some(Level::Found(est));
        }
    }
}

/// Pitch match by two-alternative forced choice in log frequency. Each trial
/// presents tones `step` octaves apart around the estimate. "Which is closer
/// to your tinnitus?" gives the sign of the error. The step halves at each
/// reversal, down to `MIN_STEP`.
#[derive(Clone, Debug)]
pub struct PitchStaircase {
    x: f64,
    step: f64,
    lo: f64,
    hi: f64,
    last_dir: i8,
    min_step_reversals: Vec<f64>,
    trials: usize,
    pub limit_hits: usize,
    pub result: Option<f64>,
}

impl PitchStaircase {
    pub const MIN_STEP_OCT: f64 = 1.0 / 6.0;
    const START_STEP_OCT: f64 = 1.0;
    const MAX_TRIALS: usize = 40;

    pub fn new(lo_hz: f64, hi_hz: f64, start_hz: f64) -> Self {
        let (lo, hi) = (lo_hz.log2(), hi_hz.log2());
        let mut s = Self {
            x: start_hz.log2(),
            step: Self::START_STEP_OCT,
            lo,
            hi,
            last_dir: 0,
            min_step_reversals: Vec::new(),
            trials: 0,
            limit_hits: 0,
            result: None,
        };
        s.clamp();
        s
    }

    /// Keeps the pair inside the range. Returns whether it had to.
    fn clamp(&mut self) -> bool {
        let (a, b) = (self.lo + self.step / 2.0, self.hi - self.step / 2.0);
        if self.x < a || self.x > b {
            self.limit_hits += 1;
            self.x = self.x.clamp(a, b);
            true
        } else {
            false
        }
    }

    /// (lower, higher) frequency of the next pair, Hz.
    pub fn pair(&self) -> (f64, f64) {
        (2f64.powf(self.x - self.step / 2.0), 2f64.powf(self.x + self.step / 2.0))
    }

    pub fn trials(&self) -> usize {
        self.trials
    }

    /// `Some(true)`: the higher tone was closer. `None`: no difference.
    pub fn respond(&mut self, higher_closer: Option<bool>) {
        if self.result.is_some() {
            return;
        }
        self.trials += 1;
        let at_min = self.step <= Self::MIN_STEP_OCT + 1e-9;
        match higher_closer {
            None => {
                if at_min {
                    self.min_step_reversals.push(self.x);
                }
                self.step = (self.step / 2.0).max(Self::MIN_STEP_OCT);
                self.last_dir = 0;
            }
            Some(up) => {
                let dir: i8 = if up { 1 } else { -1 };
                if self.last_dir != 0 && dir != self.last_dir {
                    if at_min {
                        self.min_step_reversals.push(self.x);
                    }
                    self.step = (self.step / 2.0).max(Self::MIN_STEP_OCT);
                }
                self.last_dir = dir;
                self.x += dir as f64 * self.step / 2.0;
                // Pushed against the range edge: there will never be a
                // reversal, so shrink the step as if there were one, letting
                // the pair move closer to the edge.
                if self.clamp() {
                    if at_min {
                        self.min_step_reversals.push(self.x);
                    }
                    self.step = (self.step / 2.0).max(Self::MIN_STEP_OCT);
                }
            }
        }
        self.clamp();
        if self.min_step_reversals.len() >= 4 {
            self.result = Some(2f64.powf(mean(&self.min_step_reversals)));
        } else if self.trials >= Self::MAX_TRIALS {
            let est = if self.min_step_reversals.is_empty() { self.x } else { mean(&self.min_step_reversals) };
            self.result = Some(2f64.powf(est));
        }
    }
}

/// Octave-confusion test: compare the match with the tone an octave up, and
/// if that is not preferred, with the tone an octave down. Each comparison
/// is a best-of-three vote (order randomised by the caller), because a
/// single noisy answer would move the match a whole octave. Moves at most
/// two octaves.
#[derive(Clone, Debug)]
pub struct OctaveCheck {
    pub hz: f64,
    lo: f64,
    hi: f64,
    up_phase: bool,
    votes_move: usize,
    votes_stay: usize,
    pub shift: i32,
    /// Decisions made 2-1 rather than 2-0: the octave is uncertain.
    pub split_decisions: usize,
    pub result: Option<f64>,
}

impl OctaveCheck {
    const VOTES_TO_DECIDE: usize = 2;

    pub fn new(hz: f64, lo: f64, hi: f64) -> Self {
        let mut s =
            Self { hz, lo, hi, up_phase: true, votes_move: 0, votes_stay: 0, shift: 0, split_decisions: 0, result: None };
        if 2.0 * hz > hi {
            s.go_down();
        }
        s
    }

    fn go_down(&mut self) {
        self.up_phase = false;
        if self.hz / 2.0 < self.lo {
            self.result = Some(self.hz);
        }
    }

    pub fn pair(&self) -> (f64, f64) {
        if self.up_phase { (self.hz, 2.0 * self.hz) } else { (self.hz / 2.0, self.hz) }
    }

    pub fn respond(&mut self, higher_closer: Option<bool>) {
        if self.result.is_some() {
            return;
        }
        // In the up phase "move" means the higher tone won; in the down
        // phase, the lower one. "Same" is a vote to stay.
        let wants_move = higher_closer == Some(self.up_phase);
        if wants_move { self.votes_move += 1 } else { self.votes_stay += 1 }
        if self.votes_move < Self::VOTES_TO_DECIDE && self.votes_stay < Self::VOTES_TO_DECIDE {
            return;
        }
        let decided_move = self.votes_move >= Self::VOTES_TO_DECIDE;
        if self.votes_move.min(self.votes_stay) > 0 {
            self.split_decisions += 1;
        }
        self.votes_move = 0;
        self.votes_stay = 0;
        if self.up_phase {
            if decided_move {
                self.hz *= 2.0;
                self.shift += 1;
                if self.shift >= 2 || 2.0 * self.hz > self.hi {
                    self.result = Some(self.hz);
                }
            } else if self.shift > 0 {
                self.result = Some(self.hz);
            } else {
                self.go_down();
            }
        } else if decided_move {
            self.hz /= 2.0;
            self.shift -= 1;
            if self.shift <= -2 || self.hz / 2.0 < self.lo {
                self.result = Some(self.hz);
            }
        } else {
            self.result = Some(self.hz);
        }
    }
}

/// Minimum masking level: raise the noise in 2 dB steps until the tinnitus
/// is no longer heard; two runs, the second starting 10 dB below the first
/// result; the MML is their mean.
#[derive(Clone, Debug)]
pub struct MaskingLevel {
    level: f64,
    start: f64,
    max: f64,
    runs: Vec<f64>,
    pub result: Option<Level>,
}

impl MaskingLevel {
    const STEP_DB: f64 = 2.0;

    pub fn new(start: f64, max: f64) -> Self {
        Self { level: start.min(max), start: start.min(max), max, runs: Vec::new(), result: None }
    }

    pub fn level(&self) -> f64 {
        self.level
    }

    pub fn respond(&mut self, tinnitus_audible: bool) {
        if self.result.is_some() {
            return;
        }
        if tinnitus_audible {
            if self.level >= self.max {
                self.result = Some(Level::AboveMax(self.max));
                return;
            }
            self.level = (self.level + Self::STEP_DB).min(self.max);
        } else {
            self.runs.push(self.level);
            if self.runs.len() >= 2 {
                self.result = Some(Level::Found(mean(&self.runs)));
            } else {
                self.level = (self.level - 10.0).max(self.start);
            }
        }
    }
}
