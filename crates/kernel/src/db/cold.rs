//! Cold segment placement: exact, losslessly encoded source bytes under the
//! same logical identity. Archiving moves placement and hot-cache priority,
//! never existence; a cold row keeps its anchors (minimum search keys) and a
//! route back to its bytes. Codec and dictionary are versioned so a future
//! decoder can be pinned; a roundtrip contract guards the bytes.

use rusqlite::{params, Connection, OptionalExtension};
use std::io::{Read, Write};

pub const COLD_CODEC: &str = "deflate/1";
pub const COLD_MARKER_PREFIX: &str = "[cold:";
/// Expansion cap for cold payloads. Same 2 MiB as capture intake; a deflate
/// bomb on the recall/unfold path must not allocate past this.
pub const COLD_MAX_DECODE_BYTES: usize = 2 * 1024 * 1024;

pub const COLD_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS cold_sources (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  namespace TEXT NOT NULL,
  address TEXT NOT NULL,
  codec TEXT NOT NULL,
  byte_length INTEGER NOT NULL,
  payload BLOB NOT NULL,
  context_payload BLOB,
  digest TEXT NOT NULL,
  archived_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(namespace, address)
);
"#;

pub fn ensure_cold_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(COLD_DDL)
}

pub fn encode(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(bytes)?;
    e.finish()
}

pub fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
    decode_limited(bytes, COLD_MAX_DECODE_BYTES)
}

fn decode_limited(bytes: &[u8], max_len: usize) -> Option<Vec<u8>> {
    let decoder = flate2::read::DeflateDecoder::new(bytes);
    let mut out = Vec::new();
    decoder
        .take((max_len as u64).saturating_add(1))
        .read_to_end(&mut out)
        .ok()?;
    (out.len() <= max_len).then_some(out)
}

pub fn is_cold_marker(text: &str) -> bool {
    text.starts_with(COLD_MARKER_PREFIX)
}

/// Move one legacy row's exact text (and context) into the cold segment and
/// leave a route marker in place. Idempotent; anchors/evidence stay.
pub fn move_to_cold(conn: &Connection, namespace: &str, id: i64) -> rusqlite::Result<Option<i64>> {
    ensure_cold_schema(conn)?;
    let (table, text_col) = match namespace {
        "decision" => ("decisions", "decision"),
        "memory" => ("memories", "text"),
        _ => return Ok(None),
    };
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            &format!(
                "SELECT {text_col}, {} FROM {table} WHERE id = ?1",
                if namespace == "decision" {
                    "context"
                } else {
                    "NULL"
                }
            ),
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((text, context)) = row else {
        return Ok(None);
    };
    if is_cold_marker(&text) {
        return Ok(None);
    }
    if text.len() > COLD_MAX_DECODE_BYTES
        || context
            .as_ref()
            .is_some_and(|value| value.len() > COLD_MAX_DECODE_BYTES)
    {
        return Ok(None);
    }
    let digest = cortex_logic::traces::content_hash(&text);
    let payload = encode(text.as_bytes())
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let context_payload = context
        .as_deref()
        .map(|c| encode(c.as_bytes()))
        .transpose()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "INSERT OR REPLACE INTO cold_sources (namespace, address, codec, byte_length, payload, context_payload, digest) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            namespace,
            id.to_string(),
            COLD_CODEC,
            text.len() as i64,
            payload,
            context_payload,
            digest
        ],
    )?;
    let cold_id: i64 = conn.query_row(
        "SELECT id FROM cold_sources WHERE namespace = ?1 AND address = ?2",
        params![namespace, id.to_string()],
        |r| r.get(0),
    )?;
    let marker = format!(
        "{COLD_MARKER_PREFIX}{cold_id}] {}",
        text.chars().take(60).collect::<String>()
    );
    if namespace == "decision" {
        conn.execute(
            "UPDATE decisions SET decision = ?1, context = NULL WHERE id = ?2",
            params![marker, id],
        )?;
    } else {
        conn.execute(
            "UPDATE memories SET text = ?1, tags = NULL WHERE id = ?2",
            params![marker, id],
        )?;
    }
    Ok(Some(cold_id))
}

/// Exact bytes back from the cold segment; `None` when the row was never
/// moved or the block is unreadable (reported, never silently substituted).
pub fn hydrate(
    conn: &Connection,
    namespace: &str,
    id: i64,
) -> rusqlite::Result<Option<(String, Option<String>, bool)>> {
    if !crate::db::table_exists(conn, "cold_sources") {
        return Ok(None);
    }
    let row: Option<(Vec<u8>, Option<Vec<u8>>, String, i64)> = conn
        .query_row(
            "SELECT payload, context_payload, digest, byte_length FROM cold_sources WHERE namespace = ?1 AND address = ?2",
            params![namespace, id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((payload, context_payload, digest, byte_length)) = row else {
        return Ok(None);
    };
    let Ok(expected) = usize::try_from(byte_length) else {
        return Ok(None);
    };
    if expected > COLD_MAX_DECODE_BYTES {
        return Ok(None);
    }
    let Some(bytes) = decode_limited(&payload, expected) else {
        return Ok(None);
    };
    if bytes.is_empty() && expected > 0 {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&bytes).to_string();
    let intact =
        bytes.len() as i64 == byte_length && cortex_logic::traces::content_hash(&text) == digest;
    let context = context_payload
        .and_then(|c| decode_limited(&c, COLD_MAX_DECODE_BYTES))
        .map(|c| String::from_utf8_lossy(&c).to_string());
    Ok(Some((text, context, intact)))
}

pub fn cold_count(conn: &Connection) -> i64 {
    if !crate::db::table_exists(conn, "cold_sources") {
        return 0;
    }
    conn.query_row("SELECT COUNT(*) FROM cold_sources", [], |r| r.get(0))
        .unwrap_or(0)
}
