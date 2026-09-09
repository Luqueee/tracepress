//! Fuzz-driven fragment scheduling shared by the two incremental streaming targets.

/// Largest fragment-width schedule read from an input, in bytes.
const MAX_SCHEDULE_BYTES: u8 = 32;
/// Largest scheduled fragment width, in bytes.
const MAX_CHUNK_BYTES: u8 = 64;
/// Fragment width used when an input carries no schedule.
const DEFAULT_CHUNK_BYTES: usize = 16;

/// Splits an input into a fragment-width schedule and the untouched stream body.
///
/// The leading byte declares how many schedule bytes follow. It only bounds a slice of bytes that
/// are already present, so nothing is allocated from that declared length. Keeping the body
/// contiguous lets a real SSE fixture stay parseable while the widths remain fuzzer-chosen.
pub fn split_schedule(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let (&declared, rest) = data.split_first()?;
    let length = usize::from(declared % MAX_SCHEDULE_BYTES).min(rest.len());
    rest.split_at_checked(length)
}

/// Returns the scheduled width for one fragment, always at least one byte.
pub fn width_at(schedule: &[u8], step: usize) -> usize {
    if schedule.is_empty() {
        return DEFAULT_CHUNK_BYTES;
    }
    let index = step % schedule.len();
    schedule.get(index).map_or(DEFAULT_CHUNK_BYTES, |selector| {
        usize::from(selector % MAX_CHUNK_BYTES).saturating_add(1)
    })
}
