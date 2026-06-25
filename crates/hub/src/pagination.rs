//! Bounded pagination: a sane default and a hard upper bound so a single
//! request can't ask for an unbounded result set. Ordering is supplied by each
//! query (always with an `id` tiebreak) so paging is stable.

pub const DEFAULT_LIMIT: i64 = 50;
pub const MAX_LIMIT: i64 = 200;

#[derive(Debug, Clone, Copy)]
pub struct Page {
    pub limit: i64,
    pub offset: i64,
}

impl Page {
    pub fn from(limit: Option<i64>, offset: Option<i64>) -> Self {
        Self {
            limit: limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
            offset: offset.unwrap_or(0).max(0),
        }
    }
}
