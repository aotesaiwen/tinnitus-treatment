//! Verified notch processing for tinnitus sound-therapy research.
//!
//! Music comes from the offline composer, the optional ACE-Step adapter, or
//! the listener's library. The processing pipeline does the part a music
//! model cannot guarantee: remove a precisely specified frequency band,
//! set loudness and peak level, and prove with measurements that it did so.

pub mod analysis;
pub mod conv;
pub mod fir;
pub mod generation;
pub mod loudness;
pub mod matching;
pub mod notch;
pub mod pipeline;
pub mod resample;
pub mod signals;
pub mod wav;
