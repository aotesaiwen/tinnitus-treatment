//! Resolve a selection while holding the library lock. A generated track can
//! change the order between a status response and the user's play request.

pub enum Selection {
    Current,
    Id(String),
    Offset(i32),
    AfterEnd(u64),
    IfIdle(u64),
}

pub fn resolve(
    selection: &Selection,
    ids: &[&str],
    current: Option<usize>,
    revision: u64,
    finished: bool,
    idle: bool,
) -> Result<Option<usize>, String> {
    match selection {
        Selection::AfterEnd(expected) if *expected != revision || !finished => return Ok(None),
        Selection::IfIdle(expected) if *expected != revision || !idle => return Ok(None),
        _ => {}
    }
    if ids.is_empty() {
        return Err("The music library is empty.".into());
    }
    let index = match selection {
        Selection::Id(id) => ids
            .iter()
            .position(|candidate| *candidate == id)
            .ok_or("The selected track is no longer in the library.")?,
        Selection::Current | Selection::IfIdle(_) => current.unwrap_or(0) % ids.len(),
        Selection::Offset(delta) => {
            (current.unwrap_or(0) as i64 + *delta as i64).rem_euclid(ids.len() as i64) as usize
        }
        Selection::AfterEnd(_) => current.map_or(0, |i| (i + 1) % ids.len()),
    };
    Ok(Some(index))
}
