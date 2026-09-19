//! Use-binding mint table and G1–G5. No kernel wiring.

use serde_json::json;

use crate::loc::{LocParseError, parse_loc};
use crate::store::{ByteSpan, Loc, Store};
use crate::{BindError, Error, Failure, LocNo, ObjectId, SessionId};

/// Outcome of a seed Read (path/pattern).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seed {
    pub loc: Loc,
    pub reused: bool,
}

/// Preimage for bound Edit after G1–G5.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundEdit {
    pub loc: Loc,
    pub preimage: Vec<u8>,
}

/// Session-scoped mint/bind. Grants die on `clear_bind`; locs do not.
pub struct Session<'a> {
    store: &'a Store,
    id: SessionId,
}

impl<'a> Session<'a> {
    pub fn open(store: &'a Store) -> Result<Self, Error> {
        Ok(Self {
            store,
            id: store.open_session()?,
        })
    }

    /// Resume an existing session without minting a new session row.
    pub fn attach(store: &'a Store, id: SessionId) -> Self {
        Self { store, id }
    }

    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn clear_bind(&self) -> Result<(), Error> {
        self.store.clear_grants(self.id)
    }

    fn seed_intern(
        &self,
        origin: &str,
        bytes: &[u8],
        mtime_size: &str,
        staged: bool,
    ) -> Result<(crate::store::Interned, i64), BindError> {
        let interned = self.store.intern(bytes)?;
        let seal = blake3::hash(bytes);
        Ok((
            interned,
            self.sync_path_rev(origin, interned.oid, mtime_size, seal.as_bytes(), staged)?,
        ))
    }

    /// Seed path/pattern Read. Binds. Reuses SAME-USE after freshness.
    pub fn seed_read(
        &self,
        origin: &str,
        bytes: &[u8],
        span: ByteSpan,
        batch: i64,
        mtime_size: &str,
        staged: bool,
    ) -> Result<Seed, BindError> {
        let (interned, rev) = self.seed_intern(origin, bytes, mtime_size, staged)?;
        let Some(existing) = self.store.find_same_use(self.id, origin, span, rev)? else {
            let granted = [Some("read_seed"), None][usize::from(staged)];
            let no =
                self.store
                    .insert_use(self.id, interned.oid, span, origin, rev, granted, "file")?;
            self.store.append_grant(self.id, no, batch)?;
            return Ok(Seed {
                loc: self.store.resolve(self.id, no)?,
                reused: false,
            });
        };
        existing
            .granted_by
            .is_none()
            .then_some(())
            .map(|_| self.store.set_granted_by(self.id, existing.no, "read_seed"))
            .transpose()?;
        self.store.append_grant(self.id, existing.no, batch)?;
        Ok(Seed {
            loc: self.store.resolve(self.id, existing.no)?,
            reused: true,
        })
    }

    /// `Read({target: "@N"})` — never rebinds.
    pub fn exact_read(&self, name: &str) -> Result<Loc, BindError> {
        let no = parse_loc_guard(name)?;
        self.store.resolve(self.id, no).map_err(|_| {
            BindError::Guard(Failure::new(
                "loc_unknown",
                format!("unknown loc {name}"),
                "Read",
                json!({"path": true}),
            ))
        })
    }

    /// Settled or staged Write. Staged: no path head bump, `granted_by` null.
    pub fn write_path(
        &self,
        origin: &str,
        bytes: &[u8],
        mtime_size: &str,
        staged: bool,
    ) -> Result<Loc, BindError> {
        let interned = self.store.intern(bytes)?;
        let span = ByteSpan::whole(interned.byte_len);
        let seal = blake3::hash(bytes);
        let rev = self.sync_path_rev(origin, interned.oid, mtime_size, seal.as_bytes(), staged)?;
        let granted = if staged { None } else { Some("write") };
        let no =
            self.store
                .insert_use(self.id, interned.oid, span, origin, rev, granted, "file")?;
        Ok(self.store.resolve(self.id, no)?)
    }

    /// Published path after workspace commit. Bumps generation; does not update `last` loc.
    pub fn publish_path(
        &self,
        origin: &str,
        bytes: &[u8],
        granted_by: &str,
    ) -> Result<Loc, BindError> {
        let interned = self.store.intern(bytes)?;
        let span = ByteSpan::whole(interned.byte_len);
        let seal = blake3::hash(bytes);
        let mtime_size = format!("0:{}", interned.byte_len);
        let rev = self.sync_path_rev(origin, interned.oid, &mtime_size, seal.as_bytes(), false)?;
        let no = self.store.insert_use(
            self.id,
            interned.oid,
            span,
            origin,
            rev,
            Some(granted_by),
            "file",
        )?;
        Ok(self.store.resolve(self.id, no)?)
    }

    /// G1–G5 then return preimage. `live_origin` is current path bytes (G4 re-seal).
    pub fn bind_edit(
        &self,
        explicit: Option<&str>,
        batch: i64,
        live_origin: Option<&[u8]>,
    ) -> Result<BoundEdit, BindError> {
        let loc = self.pick_edit_loc(explicit, batch)?;
        self.g3_editable(&loc)?;
        self.g4_fresh(&loc, live_origin)?;
        self.g5_grant(&loc)?;
        self.store.consume_grant(self.id, loc.no)?;
        let payload = self.store.payload(loc.oid)?;
        let start = loc.span.start as usize;
        let end = loc.span.end as usize;
        let preimage = payload.get(start..end).ok_or_else(|| {
            BindError::Guard(Failure::new(
                "not_editable",
                "selection out of object",
                "Read",
                json!({"path": loc.origin}),
            ))
        })?;
        Ok(BoundEdit {
            loc,
            preimage: preimage.to_vec(),
        })
    }

    fn pick_edit_loc(&self, explicit: Option<&str>, batch: i64) -> Result<Loc, BindError> {
        if let Some(name) = explicit {
            let no = parse_loc_guard(name)?;
            return self.store.resolve(self.id, no).map_err(|_| {
                BindError::Guard(Failure::new(
                    "loc_unknown",
                    format!("unknown loc {name}"),
                    "Read",
                    json!({"path": true}),
                ))
            });
        }
        let Some((no, grant_batch)) = self.store.latest_grant(self.id)? else {
            return Err(BindError::Guard(Failure::new(
                "unbound",
                "no seed Read in this session",
                "Read",
                json!({"path": true}),
            )));
        };
        let in_batch = self.store.grants_in_batch(self.id, grant_batch)?;
        let mut distinct = in_batch.clone();
        distinct.sort_unstable();
        distinct.dedup();
        let _ = batch;
        if distinct.len() >= 2 {
            return Err(BindError::Guard(Failure::new(
                "bind_ambiguous",
                format!("distinct locs in batch: {distinct:?}"),
                "Edit",
                json!({"loc": format!("@{}", distinct[0])}),
            )));
        }
        Ok(self.store.resolve(self.id, no)?)
    }

    fn g3_editable(&self, loc: &Loc) -> Result<(), BindError> {
        if loc.kind != "file" || (loc.granted_by.is_none() && loc.origin.is_empty()) {
            return Err(BindError::Guard(Failure::new(
                "not_editable",
                "loc is not an editable file origin",
                "Read",
                json!({"path": loc.origin}),
            )));
        }
        Ok(())
    }

    fn g4_fresh(&self, loc: &Loc, live_origin: Option<&[u8]>) -> Result<(), BindError> {
        let Some(head) = self.store.path_head(&loc.origin)? else {
            return Ok(());
        };
        if loc.rev != head.0 {
            return stale(&loc.origin);
        }
        if let Some(live) = live_origin {
            let live_seal = blake3::hash(live);
            if live_seal.as_bytes().as_slice() != head.3.as_slice() {
                return stale(&loc.origin);
            }
        }
        Ok(())
    }

    fn g5_grant(&self, loc: &Loc) -> Result<(), BindError> {
        match loc.granted_by.as_deref() {
            Some("read_seed" | "write" | "edit") => Ok(()),
            _ => Err(BindError::Guard(Failure::new(
                "capsule_not_authority",
                "loc has no Edit grant",
                "Read",
                json!({"path": loc.origin}),
            ))),
        }
    }

    fn sync_path_rev(
        &self,
        origin: &str,
        oid: ObjectId,
        mtime_size: &str,
        seal: &[u8],
        staged: bool,
    ) -> Result<i64, Error> {
        let true = !staged else {
            return Ok(self.store.path_head(origin)?.map(|h| h.0).unwrap_or(0));
        };
        match self.store.path_head(origin)? {
            None => {
                self.store.set_path_head(origin, 1, oid, mtime_size, seal)?;
                Ok(1)
            }
            Some((rev, _, ms, old_seal)) => {
                if unchanged_path_seal(&ms, mtime_size, &old_seal, seal) {
                    return Ok(rev);
                }
                let next = rev + 1;
                self.store
                    .set_path_head(origin, next, oid, mtime_size, seal)?;
                Ok(next)
            }
        }
    }
}

fn unchanged_path_seal(ms: &str, mtime_size: &str, old_seal: &[u8], seal: &[u8]) -> bool {
    [ms == mtime_size, old_seal == seal].iter().all(|ok| *ok)
}

fn parse_loc_guard(name: &str) -> Result<LocNo, BindError> {
    match parse_loc(name) {
        Ok(n) => Ok(n),
        Err(LocParseError::NotALoc) if crate::is_digest_spelling(name) => Err(BindError::Guard(
            Failure::new("not_a_loc", "not a loc", "Read", json!({"path": true})),
        )),
        Err(LocParseError::NotALoc) => Err(BindError::Guard(Failure::new(
            "not_a_loc",
            format!("{name} is not a loc"),
            "Read",
            json!({"path": name}),
        ))),
    }
}

fn stale(origin: &str) -> Result<(), BindError> {
    Err(BindError::Guard(Failure::new(
        "bind_stale",
        "origin moved; loc is historical",
        "Read",
        json!({"path": origin}),
    )))
}
