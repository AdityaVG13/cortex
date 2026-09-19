# cortex-refzero — contract

**Purpose / layer.** The Cortex identity store: interned byte objects and
captured-use locs, plus the portable `z://blob/` grammar. Port of ZeroStack
`refzero-core` at `2e220e1` + `522a814` + `39ae0fd` with byte-identical
semantics. `cortex-kernel` composes it for the session-owned sidecar;
no engine depends on it directly.

**Public types.** `Store` (`open`, `intern`, `intern_with_suite`,
`open_session`, `resume_or_open_session`, `freeze_session`, `session_status`,
`high_water`, `loc_quota` / `set_loc_quota`, `expose`, `expose_imported`,
`resolve`, `payload`, `clear_grants`; `page` / `bind_edit` / `export` /
`import` are `NotYetImplemented` stubs), `Session` (`open`, `attach`,
`seed_read`, `exact_read`, `write_path`, `publish_path`, `bind_edit`,
`clear_bind`), `RecallEnvelope` (`{oid_hex, start, end}`),
`import` (`ImportRequest` → `Imported`), `parse_loc` / `format_loc` /
`is_digest_spelling`, `identity_slot_hits`, `zeroref::{ZeroRef,
ZeroFragment, ZeroRefErrorClass, …}`, `digest::{digest32, digest_hex,
contract_digest, …}`. `ObjectId` / `SessionId` are UUIDv4; `LocNo` is `u64`.

**Invariants.** Interned objects dedup on (seal suite, BLAKE3 seal,
byte length, payload bytes): same bytes one oid, same seal+length with
different bytes is `IntegrityCollision`. Oids are allocated UUIDv4, never
content digests. Locs are commit-before-reveal serials starting at 1 per
session; an aborted bump never recycles a number; the high-water mark is
monotonic across sessions. First loc is `@1`. Quota defaults to
`MAX_SAFE_INTEGER`. Digest spellings (`z://blob/…`, raw 64-hex) are never
locs: identity slots holding them fail closed as `not_a_loc`.

**Error model.** `Error` (store faults: `UnknownSession`,
`SessionInactive`, `UnknownLoc`, `UnknownObject`, `InvalidSpan`,
`IntegrityCollision`, `ImportConflict`, `DigestMismatch`, `Fragment`,
`Quota`, …) and `BindError::Guard(Failure)` for G1–G5 (`unbound`,
`not_a_loc`, `loc_unknown`, `bind_ambiguous`, `bind_stale`,
`not_editable`, `capsule_not_authority`). `Failure` carries a teaching
`next` call. Structured `zeroref::ZeroRefError` classes mirror the shared
fixtures verbatim.

**Determinism.** Deterministic given store state and inputs, except
allocated UUIDv4 oids/session ids (CSPRNG). Digests are BLAKE3-only.

**Cancellation.** Synchronous SQLite; no async, no cancellation tokens.
`BEGIN IMMEDIATE` transactions; failures roll back.

**Unsafe.** None (`forbid(unsafe_code)`).

**Feature flags.** None.

**Port deltas vs the reference** (semantics unchanged, drivers aligned):
fsqlite → rusqlite (bundled), so Cortex carries one sqlite stack; manual
UUIDv4 via the workspace `uuid` crate instead of raw `getrandom`;
hand-written `Display`/`Error` impls instead of `thiserror`; no inline
`#[cfg(test)]` (repo policy: contracts live in `tests/`); crate renamed to
`cortex-refzero` so a joint harness never links two `refzero-core` crates.
The SQLite DDL is byte-identical to the reference.

**Conformance tests.** `tests/contracts/refzero_identity.rs`,
`tests/contracts/refzero_interop.rs` (+ vendored
`tests/fixtures/zeroref-fixtures.json`).

**No-claim boundaries.** Not a machine-wide service: the store file is
session-owned next to the brain DB. `page`/`bind_edit`/`export`/`import`
store stubs are unimplemented by design (mint/bind lives on `Session`).
