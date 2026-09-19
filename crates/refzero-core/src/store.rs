use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::ids::{ObjectId, SessionId, blob_to_id, hex as hex_id};
use crate::{Error, LocNo};

/// Tagged integrity suite. Changing this must not change object ids or loc numbers.
pub const DEFAULT_SEAL_SUITE: &str = "blake3/1";

const SCHEMA: &str = "
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA busy_timeout = 5000;
CREATE TABLE IF NOT EXISTS objects (
    oid BLOB(16) PRIMARY KEY,
    byte_len INTEGER NOT NULL,
    seal_suite TEXT NOT NULL,
    seal BLOB NOT NULL,
    payload BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS objects_seal_candidate
    ON objects (seal_suite, seal, byte_len);
CREATE TABLE IF NOT EXISTS object_aliases (
    alias_oid BLOB(16) PRIMARY KEY,
    canonical_oid BLOB(16) NOT NULL REFERENCES objects(oid),
    CHECK (alias_oid != canonical_oid)
);
CREATE TABLE IF NOT EXISTS sessions (
    session_id BLOB(16) PRIMARY KEY,
    next_no INTEGER NOT NULL CHECK (next_no > 0),
    status TEXT NOT NULL CHECK (status IN ('active', 'frozen', 'closed'))
);
CREATE TABLE IF NOT EXISTS locs (
    session_id BLOB(16) NOT NULL REFERENCES sessions(session_id),
    no INTEGER NOT NULL CHECK (no > 0),
    oid BLOB(16) NOT NULL REFERENCES objects(oid),
    selection_start INTEGER NOT NULL,
    selection_end INTEGER NOT NULL,
    role TEXT NOT NULL,
    workspace_id TEXT,
    source_path TEXT,
    source_revision TEXT,
    source_cell TEXT,
    origin TEXT NOT NULL DEFAULT '',
    rev INTEGER NOT NULL DEFAULT 0,
    granted_by TEXT,
    kind TEXT NOT NULL DEFAULT 'file',
    PRIMARY KEY (session_id, no),
    CHECK (selection_start <= selection_end)
);
CREATE TABLE IF NOT EXISTS paths (
    origin TEXT PRIMARY KEY,
    cur_rev INTEGER NOT NULL,
    oid BLOB(16) NOT NULL REFERENCES objects(oid),
    mtime_size TEXT NOT NULL,
    origin_seal BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS grants (
    session_id BLOB(16) NOT NULL REFERENCES sessions(session_id),
    seq INTEGER NOT NULL,
    loc_no INTEGER NOT NULL,
    batch INTEGER NOT NULL,
    PRIMARY KEY (session_id, seq)
);
CREATE TABLE IF NOT EXISTS import_map (
    producer TEXT NOT NULL,
    algorithm TEXT NOT NULL,
    locator TEXT NOT NULL,
    oid BLOB(16) NOT NULL REFERENCES objects(oid),
    PRIMARY KEY (producer, algorithm, locator)
);
CREATE TABLE IF NOT EXISTS store_meta (
    k TEXT PRIMARY KEY,
    v TEXT NOT NULL
);
";

/// Half-open byte range on an interned object. Whole object is `0..len`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteSpan {
    pub start: u64,
    pub end: u64,
}

impl ByteSpan {
    pub fn whole(len: u64) -> Self {
        Self { start: 0, end: len }
    }
}

/// Result of intern: public oid is never the seal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interned {
    pub oid: ObjectId,
    pub byte_len: u64,
}

/// Captured use. Immutable after commit-before-reveal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loc {
    pub no: LocNo,
    pub oid: ObjectId,
    pub span: ByteSpan,
    pub role: String,
    pub source_path: Option<String>,
    pub origin: String,
    pub rev: i64,
    pub granted_by: Option<String>,
    pub kind: String,
}

/// Daemonless session-owned SQLite identity store. Not a machine-wide service.
pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn intern(&self, bytes: &[u8]) -> Result<Interned, Error> {
        self.intern_with_suite(bytes, DEFAULT_SEAL_SUITE)
    }

    /// Makeup hook: changing the hidden checksum suite must not change loc numbers
    /// or the oid of an already interned object.
    pub fn intern_with_suite(&self, bytes: &[u8], suite: &str) -> Result<Interned, Error> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = intern_in_conn_suite(&self.conn, bytes, suite);
        finish_txn(&self.conn, result)
    }

    /// Cross-product adoption: insert `bytes` under a sender-chosen `oid`
    /// (e.g. from a [`RecallEnvelope`](crate::RecallEnvelope)) so identities
    /// survive a hop between stores. Extension beyond the reference bead;
    /// the collision rule matches [`Store::intern`]: same oid with different
    /// bytes, or same seal+length with different bytes, is
    /// [`Error::IntegrityCollision`]. Idempotent for the same (oid, bytes).
    pub fn adopt_object(&self, oid: ObjectId, bytes: &[u8]) -> Result<Interned, Error> {
        self.adopt_object_with_suite(oid, bytes, DEFAULT_SEAL_SUITE)
    }

    /// Suite-parameterized [`Store::adopt_object`].
    pub fn adopt_object_with_suite(
        &self,
        oid: ObjectId,
        bytes: &[u8],
        suite: &str,
    ) -> Result<Interned, Error> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = adopt_in_conn_suite(&self.conn, oid, bytes, suite);
        finish_txn(&self.conn, result)
    }

    pub fn open_session(&self) -> Result<SessionId, Error> {
        let id = SessionId::new_v4()?;
        let next = self.high_water()?.max(1);
        self.conn.execute(
            "INSERT INTO sessions (session_id, next_no, status) VALUES (?1, ?2, 'active')",
            params![id.as_bytes().as_slice(), next as i64],
        )?;
        self.set_meta("last_session", &hex_id(id.as_bytes()))?;
        Ok(id)
    }

    /// Resume the last session if present; otherwise open a new one.
    /// Missing last-session never restarts numbering at `@1` when history exists.
    pub fn resume_or_open_session(&self) -> Result<SessionId, Error> {
        if let Some(hex) = self.meta("last_session")? {
            if let Ok(id) = session_from_hex(&hex) {
                if self.session_status(id)?.is_some() {
                    return Ok(id);
                }
            }
        }
        self.open_session()
    }

    pub fn freeze_session(&self, id: SessionId) -> Result<(), Error> {
        let n = self.conn.execute(
            "UPDATE sessions SET status = 'frozen' WHERE session_id = ?1",
            params![id.as_bytes().as_slice()],
        )?;
        if n == 0 {
            return Err(Error::UnknownSession);
        }
        Ok(())
    }

    pub fn session_status(&self, id: SessionId) -> Result<Option<String>, Error> {
        Ok(self
            .conn
            .query_row(
                "SELECT status FROM sessions WHERE session_id = ?1",
                params![id.as_bytes().as_slice()],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn high_water(&self) -> Result<u64, Error> {
        if let Some(v) = self.meta("loc_high_water")? {
            if let Ok(n) = v.parse::<u64>() {
                return Ok(n.max(1));
            }
        }
        let n: i64 =
            self.conn
                .query_row("SELECT COALESCE(MAX(next_no), 1) FROM sessions", [], |r| {
                    r.get(0)
                })?;
        Ok((n.max(1)) as u64)
    }

    pub fn set_loc_quota(&self, quota: u64) -> Result<(), Error> {
        self.set_meta("loc_quota", &quota.to_string())
    }

    pub fn loc_quota(&self) -> Result<u64, Error> {
        Ok(self
            .meta("loc_quota")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(crate::MAX_SAFE_INTEGER))
    }

    fn meta(&self, k: &str) -> Result<Option<String>, Error> {
        Ok(self
            .conn
            .query_row("SELECT v FROM store_meta WHERE k = ?1", params![k], |r| {
                r.get(0)
            })
            .optional()?)
    }

    fn set_meta(&self, k: &str, v: &str) -> Result<(), Error> {
        self.conn.execute(
            "INSERT INTO store_meta (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![k, v],
        )?;
        Ok(())
    }

    /// Expose an imported object with **no** Edit grant.
    pub fn expose_imported(
        &self,
        session: SessionId,
        oid: ObjectId,
        span: ByteSpan,
    ) -> Result<LocNo, Error> {
        self.insert_use(session, oid, span, "", 0, None, "import")
    }

    /// Allocate a loc, COMMIT, then return the number. Never AUTOINCREMENT rowid.
    pub fn expose(
        &self,
        session: SessionId,
        oid: ObjectId,
        span: ByteSpan,
        role: &str,
        source_path: Option<&str>,
    ) -> Result<LocNo, Error> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = expose_in_conn(&self.conn, session, oid, span, role, source_path);
        finish_txn(&self.conn, result)
    }

    pub fn resolve(&self, session: SessionId, no: LocNo) -> Result<Loc, Error> {
        self.conn
            .query_row(
                "SELECT oid, selection_start, selection_end, role, source_path,
                    origin, rev, granted_by, kind
             FROM locs WHERE session_id = ?1 AND no = ?2",
                params![session.as_bytes().as_slice(), no as i64],
                |row| {
                    let oid_bytes: Vec<u8> = row.get(0)?;
                    let oid =
                        ObjectId::from_bytes(blob_to_id(&oid_bytes).map_err(|e| error_to_sql(e))?);
                    Ok(Loc {
                        no,
                        oid,
                        span: ByteSpan {
                            start: row.get::<_, i64>(1)? as u64,
                            end: row.get::<_, i64>(2)? as u64,
                        },
                        role: row.get(3)?,
                        source_path: row.get(4)?,
                        origin: row.get(5).unwrap_or_default(),
                        rev: row.get(6).unwrap_or(0),
                        granted_by: row.get(7)?,
                        kind: row.get(8).unwrap_or_else(|_| "file".to_string()),
                    })
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Error::UnknownLoc(no),
                rusqlite::Error::ToSqlConversionFailure(boxed) => boxed
                    .downcast::<Error>()
                    .map(|e| *e)
                    .unwrap_or(Error::UnknownObject),
                other => Error::Sqlite(other),
            })
    }

    pub fn payload(&self, oid: ObjectId) -> Result<Vec<u8>, Error> {
        self.conn
            .query_row(
                "SELECT payload FROM objects WHERE oid = ?1",
                params![oid.as_bytes().as_slice()],
                |r| r.get(0),
            )
            .map_err(|_| Error::UnknownObject)
    }

    pub fn page(&self) -> Result<(), Error> {
        Err(Error::NotYetImplemented("page"))
    }

    pub fn bind_edit(&self) -> Result<(), Error> {
        Err(Error::NotYetImplemented("bind_edit"))
    }

    pub fn export(&self) -> Result<(), Error> {
        Err(Error::NotYetImplemented("export"))
    }

    pub fn import(&self) -> Result<(), Error> {
        Err(Error::NotYetImplemented("import"))
    }

    /// Test helper: bump `next_no` then ROLLBACK. Proves loc serials are not AUTOINCREMENT.
    #[doc(hidden)]
    pub fn __test_bump_next_no_then_rollback(&self, session: SessionId) -> Result<(), Error> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = next_loc_no(&self.conn, session);
        let _ = self.conn.execute_batch("ROLLBACK");
        result.map(|_| ())
    }

    /// Test helper: overwrite payload, keep seal+len so intern of original bytes collides.
    #[doc(hidden)]
    pub fn __test_overwrite_payload_keep_seal(
        &self,
        oid: ObjectId,
        payload: &[u8],
    ) -> Result<(), Error> {
        let n = self.conn.execute(
            "UPDATE objects SET payload = ?1 WHERE oid = ?2 AND byte_len = ?3",
            params![payload, oid.as_bytes().as_slice(), payload.len() as i64],
        )?;
        if n != 1 {
            return Err(Error::UnknownObject);
        }
        Ok(())
    }

    pub(crate) fn write_txn<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, Error>,
    ) -> Result<T, Error> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        finish_txn(&self.conn, f(&self.conn))
    }

    pub(crate) fn find_same_use(
        &self,
        session: SessionId,
        origin: &str,
        span: ByteSpan,
        rev: i64,
    ) -> Result<Option<Loc>, Error> {
        let no: Option<i64> = self
            .conn
            .query_row(
                "SELECT no FROM locs
             WHERE session_id = ?1 AND origin = ?2 AND selection_start = ?3
               AND selection_end = ?4 AND rev = ?5",
                params![
                    session.as_bytes().as_slice(),
                    origin,
                    span.start as i64,
                    span.end as i64,
                    rev
                ],
                |r| r.get(0),
            )
            .optional()?;
        match no {
            None => Ok(None),
            Some(no) => self.resolve(session, no as LocNo).map(Some),
        }
    }

    pub(crate) fn insert_use(
        &self,
        session: SessionId,
        oid: ObjectId,
        span: ByteSpan,
        origin: &str,
        rev: i64,
        granted_by: Option<&str>,
        kind: &str,
    ) -> Result<LocNo, Error> {
        self.write_txn(|conn| {
            let no = next_loc_no(conn, session)?;
            conn.execute(
                "INSERT INTO locs (
                    session_id, no, oid, selection_start, selection_end, role, source_path,
                    origin, rev, granted_by, kind
                ) VALUES (?1, ?2, ?3, ?4, ?5, 'file', ?6, ?6, ?7, ?8, ?9)",
                params![
                    session.as_bytes().as_slice(),
                    no as i64,
                    oid.as_bytes().as_slice(),
                    span.start as i64,
                    span.end as i64,
                    origin,
                    rev,
                    granted_by,
                    kind
                ],
            )?;
            Ok(no)
        })
    }

    pub(crate) fn path_head(
        &self,
        origin: &str,
    ) -> Result<Option<(i64, ObjectId, String, Vec<u8>)>, Error> {
        let row: Option<(i64, Vec<u8>, String, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT cur_rev, oid, mtime_size, origin_seal FROM paths WHERE origin = ?1",
                params![origin],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        match row {
            None => Ok(None),
            Some((rev, oid_bytes, ms, seal)) => Ok(Some((
                rev,
                ObjectId::from_bytes(blob_to_id(&oid_bytes)?),
                ms,
                seal,
            ))),
        }
    }

    pub(crate) fn set_path_head(
        &self,
        origin: &str,
        rev: i64,
        oid: ObjectId,
        mtime_size: &str,
        seal: &[u8],
    ) -> Result<(), Error> {
        self.conn.execute(
            "INSERT INTO paths (origin, cur_rev, oid, mtime_size, origin_seal)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(origin) DO UPDATE SET
                cur_rev = excluded.cur_rev,
                oid = excluded.oid,
                mtime_size = excluded.mtime_size,
                origin_seal = excluded.origin_seal",
            params![origin, rev, oid.as_bytes().as_slice(), mtime_size, seal],
        )?;
        Ok(())
    }

    pub(crate) fn set_granted_by(
        &self,
        session: SessionId,
        no: LocNo,
        granted_by: &str,
    ) -> Result<(), Error> {
        self.conn.execute(
            "UPDATE locs SET granted_by = ?1
             WHERE session_id = ?2 AND no = ?3 AND granted_by IS NULL",
            params![granted_by, session.as_bytes().as_slice(), no as i64],
        )?;
        Ok(())
    }

    pub(crate) fn append_grant(
        &self,
        session: SessionId,
        no: LocNo,
        batch: i64,
    ) -> Result<(), Error> {
        let seq: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM grants WHERE session_id = ?1",
            params![session.as_bytes().as_slice()],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO grants (session_id, seq, loc_no, batch) VALUES (?1, ?2, ?3, ?4)",
            params![session.as_bytes().as_slice(), seq + 1, no as i64, batch],
        )?;
        Ok(())
    }

    pub fn clear_grants(&self, session: SessionId) -> Result<(), Error> {
        self.conn.execute(
            "DELETE FROM grants WHERE session_id = ?1",
            params![session.as_bytes().as_slice()],
        )?;
        Ok(())
    }

    pub(crate) fn consume_grant(&self, session: SessionId, no: LocNo) -> Result<(), Error> {
        self.conn.execute(
            "DELETE FROM grants WHERE session_id = ?1 AND loc_no = ?2",
            params![session.as_bytes().as_slice(), no as i64],
        )?;
        Ok(())
    }

    pub(crate) fn latest_grant(&self, session: SessionId) -> Result<Option<(LocNo, i64)>, Error> {
        let row: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT loc_no, batch FROM grants WHERE session_id = ?1 ORDER BY seq DESC LIMIT 1",
                params![session.as_bytes().as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.map(|(no, batch)| (no as LocNo, batch)))
    }

    pub(crate) fn grants_in_batch(
        &self,
        session: SessionId,
        batch: i64,
    ) -> Result<Vec<LocNo>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT loc_no FROM grants WHERE session_id = ?1 AND batch = ?2 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![session.as_bytes().as_slice(), batch], |r| {
            r.get::<_, i64>(0)
        })?;
        let mut out = Vec::new();
        for no in rows {
            out.push(no? as LocNo);
        }
        Ok(out)
    }

    pub(crate) fn lookup_import(
        &self,
        producer: &str,
        algorithm: &str,
        locator: &str,
    ) -> Result<Option<ObjectId>, Error> {
        let oid_bytes: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT oid FROM import_map
             WHERE producer = ?1 AND algorithm = ?2 AND locator = ?3",
                params![producer, algorithm, locator],
                |r| r.get(0),
            )
            .optional()?;
        match oid_bytes {
            None => Ok(None),
            Some(bytes) => Ok(Some(ObjectId::from_bytes(blob_to_id(&bytes)?))),
        }
    }

    pub(crate) fn insert_import(
        &self,
        producer: &str,
        algorithm: &str,
        locator: &str,
        oid: ObjectId,
    ) -> Result<(), Error> {
        self.conn.execute(
            "INSERT INTO import_map (producer, algorithm, locator, oid)
             VALUES (?1, ?2, ?3, ?4)",
            params![producer, algorithm, locator, oid.as_bytes().as_slice()],
        )?;
        Ok(())
    }
}

fn error_to_sql(e: Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

fn finish_txn<T>(conn: &Connection, result: Result<T, Error>) -> Result<T, Error> {
    match result {
        Ok(v) => {
            conn.execute_batch("COMMIT")?;
            Ok(v)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

fn intern_in_conn_suite(conn: &Connection, bytes: &[u8], suite: &str) -> Result<Interned, Error> {
    let seal = blake3::hash(bytes);
    if let Some(oid) = existing_object_suite(conn, suite, seal.as_bytes(), bytes)? {
        return Ok(Interned {
            oid,
            byte_len: bytes.len() as u64,
        });
    }
    let oid = insert_new_object_suite(conn, bytes, suite, seal.as_bytes())?;
    Ok(Interned {
        oid,
        byte_len: bytes.len() as u64,
    })
}

fn adopt_in_conn_suite(
    conn: &Connection,
    oid: ObjectId,
    bytes: &[u8],
    suite: &str,
) -> Result<Interned, Error> {
    let seal = blake3::hash(bytes);
    let existing: Option<Vec<u8>> = conn
        .query_row(
            "SELECT payload FROM objects WHERE oid = ?1",
            params![oid.as_bytes().as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(payload) = existing {
        if payload.as_slice() == bytes {
            return Ok(Interned {
                oid,
                byte_len: bytes.len() as u64,
            });
        }
        return Err(Error::IntegrityCollision);
    }
    // Same seal+length with different bytes anywhere in the suite collides,
    // exactly as in intern. Same bytes under another oid is an explicit
    // alias: the sender's oid is still inserted below.
    let _ = existing_object_suite(conn, suite, seal.as_bytes(), bytes)?;
    conn.execute(
        "INSERT INTO objects (oid, byte_len, seal_suite, seal, payload)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            oid.as_bytes().as_slice(),
            bytes.len() as i64,
            suite,
            seal.as_bytes().as_slice(),
            bytes
        ],
    )?;
    Ok(Interned {
        oid,
        byte_len: bytes.len() as u64,
    })
}

fn expose_in_conn(
    conn: &Connection,
    session: SessionId,
    oid: ObjectId,
    span: ByteSpan,
    role: &str,
    source_path: Option<&str>,
) -> Result<LocNo, Error> {
    let len = object_len(conn, oid)?;
    if span.start > span.end || span.end > len {
        return Err(Error::InvalidSpan {
            start: span.start,
            end: span.end,
            len,
        });
    }
    let no = next_loc_no(conn, session)?;
    conn.execute(
        "INSERT INTO locs (
            session_id, no, oid, selection_start, selection_end, role, source_path,
            origin, rev, granted_by, kind
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 'read_seed', 'file')",
        params![
            session.as_bytes().as_slice(),
            no as i64,
            oid.as_bytes().as_slice(),
            span.start as i64,
            span.end as i64,
            role,
            source_path,
            source_path.unwrap_or("")
        ],
    )?;
    Ok(no)
}

fn existing_object_suite(
    conn: &Connection,
    suite: &str,
    seal: &[u8],
    bytes: &[u8],
) -> Result<Option<ObjectId>, Error> {
    let mut stmt = conn.prepare(
        "SELECT oid, payload FROM objects
         WHERE seal_suite = ?1 AND seal = ?2 AND byte_len = ?3",
    )?;
    let rows = stmt.query_map(params![suite, seal, bytes.len() as i64], |r| {
        Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
    })?;
    let mut matched = None;
    for row in rows {
        let (oid_bytes, payload) = row?;
        if payload.as_slice() == bytes {
            matched = Some(ObjectId::from_bytes(blob_to_id(&oid_bytes)?));
        } else {
            return Err(Error::IntegrityCollision);
        }
    }
    Ok(matched)
}

fn insert_new_object_suite(
    conn: &Connection,
    bytes: &[u8],
    suite: &str,
    seal: &[u8],
) -> Result<ObjectId, Error> {
    for _ in 0..8 {
        let oid = ObjectId::new_v4()?;
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO objects (oid, byte_len, seal_suite, seal, payload)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                oid.as_bytes().as_slice(),
                bytes.len() as i64,
                suite,
                seal,
                bytes
            ],
        )?;
        if inserted == 1 {
            return Ok(oid);
        }
    }
    Err(Error::Entropy)
}

fn object_len(conn: &Connection, oid: ObjectId) -> Result<u64, Error> {
    let len: Option<i64> = conn
        .query_row(
            "SELECT byte_len FROM objects WHERE oid = ?1",
            params![oid.as_bytes().as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    len.map(|n| n as u64).ok_or(Error::UnknownObject)
}

fn next_loc_no(conn: &Connection, session: SessionId) -> Result<LocNo, Error> {
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM sessions WHERE session_id = ?1",
            params![session.as_bytes().as_slice()],
            |r| r.get(0),
        )
        .optional()?;
    let true = status.as_deref() == Some("active") else {
        return Err(status
            .is_some()
            .then_some(Error::SessionInactive)
            .unwrap_or(Error::UnknownSession));
    };
    let quota = loc_quota_in(conn)?;
    let upcoming: i64 = conn.query_row(
        "SELECT next_no FROM sessions WHERE session_id = ?1",
        params![session.as_bytes().as_slice()],
        |r| r.get(0),
    )?;
    let upcoming = upcoming as u64;
    let false = upcoming > quota else {
        return Err(Error::Quota);
    };
    let no: i64 = conn.query_row(
        "UPDATE sessions SET next_no = next_no + 1
         WHERE session_id = ?1 AND status = 'active'
         RETURNING next_no - 1",
        params![session.as_bytes().as_slice()],
        |r| r.get(0),
    )?;
    let false = no < 1 else {
        return Err(Error::SessionInactive);
    };
    let issued = no as LocNo;
    let high = issued.saturating_add(1);
    conn.execute(
        "INSERT INTO store_meta (k, v) VALUES ('loc_high_water', ?1)
         ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        params![high.to_string()],
    )?;
    Ok(issued)
}

fn loc_quota_in(conn: &Connection) -> Result<u64, Error> {
    let v: Option<String> = conn
        .query_row("SELECT v FROM store_meta WHERE k = 'loc_quota'", [], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(v.and_then(|v| v.parse().ok())
        .unwrap_or(crate::MAX_SAFE_INTEGER))
}

fn session_from_hex(hex: &str) -> Result<SessionId, Error> {
    if hex.len() != 32 {
        return Err(Error::UnknownSession);
    }
    let mut bytes = [0u8; 16];
    for i in 0..16 {
        bytes[i] =
            u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| Error::UnknownSession)?;
    }
    Ok(SessionId::from_bytes(bytes))
}
