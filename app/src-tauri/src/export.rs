//! Export a verified listening copy, leaving the reusable source untouched.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tmdsp::{pipeline, wav};

pub fn destination(path: &Path, library: &[PathBuf]) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
    {
        return Err("Choose an absolute path ending in .wav.".into());
    }
    let file = path.file_name().ok_or("Choose a WAV file name.")?;
    let parent = path
        .parent()
        .ok_or("Choose an output folder.")?
        .canonicalize()
        .map_err(|_| "The output folder is unavailable.")?;
    let path = parent.join(file);
    let resolved = path.canonicalize().unwrap_or_else(|_| path.clone());
    if library.iter().any(|source| {
        let canonical = source.canonicalize().unwrap_or_else(|_| source.clone());
        canonical == resolved || source.is_dir() && resolved.starts_with(canonical)
    }) {
        return Err("Choose a location outside your source music folders. Library sources must stay unprocessed.".into());
    }
    Ok(path)
}

pub fn prepare(
    source: &Path,
    opts: &pipeline::Options,
) -> Result<(Vec<u8>, pipeline::Report), String> {
    let source_bytes =
        std::fs::read(source).map_err(|e| format!("Could not read the source: {e}"))?;
    let audio = wav::parse_wav(&source_bytes).map_err(|e| e.to_string())?;
    let (bytes, mut report) =
        pipeline::run(&audio, opts).map_err(|e| format!("Audio check failed: {e}"))?;
    if !report.all_pass || !report.warnings.is_empty() {
        return Err("This track did not pass all audio checks for your current settings. Create another variation before exporting.".into());
    }
    report.input_sha256 = Some(format!("{:x}", Sha256::digest(&source_bytes)));
    report.input_path = Some(source.display().to_string());
    Ok((bytes, report))
}
