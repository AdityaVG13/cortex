pub(crate) fn rebuild_clock_projections_locked(conn: &rusqlite::Connection, batch_size: usize) -> Result<usize, String> {
    crate::clockwork::rebuild_clock_projections(conn, batch_size).map_err(|e| e.to_string())
}
