//! Tinnitus matching: pitch (with octave-confusion check), loudness, and
//! minimum masking level, following the standard audiologic procedure.
//! The output feeds the notch processor (`recommended_notch.center_hz`).

pub mod procedures;
pub mod session;
pub mod sim;
pub mod stimulus;

pub use procedures::Level;
pub use session::{Answer, Ear, MatchReport, Prompt, Question, Session, SessionConfig, Stage, TinnitusSound};
pub use stimulus::{Calibration, Renderer, Sound, Stimulus};
