//! Verified notch processing for tinnitus sound-therapy research.
//!
//! The music itself comes from elsewhere (an open generative model such as
//! ACE-Step 1.5, or the listener's own library). This crate does the part a
//! music model cannot guarantee: remove a precisely specified frequency band,
//! set loudness and peak level, and prove with measurements that it did so.

pub mod analysis;
pub mod conv;
pub mod fir;
pub mod loudness;
pub mod matching;
pub mod notch;
pub mod pipeline;
pub mod resample;
pub mod signals;
pub mod wav;
