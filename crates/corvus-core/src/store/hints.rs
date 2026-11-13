//! Wake-up hints (D0016): written only by triggers, read by Hermod, pruned by housekeeping.

use corvus_domain::Result;
use rusqlite::params;

use crate::db::{Reader, SqlResultExt, WriteTx};

/// Highest hint id (0 when empty).
pub fn max_id(r: &impl Reader) -> Result<i64> {
    r.sql()
        .query_row("SELECT coalesce(max(id), 0) FROM hints", [], |r| r.get(0))
        .db()
}

/// Hints after `cursor`, oldest first.
pub fn after(r: &impl Reader, cursor: i64) -> Result<Vec<(i64, String)>> {
    let mut st = r
        .sql()
        .prepare("SELECT id, topic FROM hints WHERE id > ?1 ORDER BY id")
        .db()?;
    let rows = st.query_map([cursor], |r| Ok((r.get(0)?, r.get(1)?))).db()?;
    rows.collect::<rusqlite::Result<_>>().db()
}

/// Keeps the newest `keep` rows. Returns how many were removed.
pub fn prune(tx: &WriteTx<'_>, keep: i64) -> Result<usize> {
    tx.sql_mut()
        .execute(
            "DELETE FROM hints WHERE id <= (SELECT coalesce(max(id), 0) FROM hints) - ?1",
            params![keep],
        )
        .db()
}
