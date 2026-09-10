# cortex-logic — CONTRACT

**Purpose / layer.** Pure, deterministic semantics: protocol vocabulary
(`protocol`), Clock-Quorum projections and admission (`clockwork`), identity
graph (`graph`), provenance traces/HEAD (`traces`), conflict classes
(`conflict`), budgets, rate limits, API request types. Depends on no HTTP
library, process supervision or model runtime.

**Public types.** `protocol::{LogicalId, ExactRef, Locator, Integrity,
Frontier, Principal, ResponseStatus, Envelope, Receipt, DurabilityVector,
AckProfile, PayloadAvailability}`; `clockwork::{QueryFrame, ClockEvidence,
RankKey, admit}`; `api_types::{StoreRequest, RetentionClass, …}`.

**Invariants.**
- Identity kinds are distinct types; none converts implicitly into another.
- `ResponseStatus` is the single status vocabulary; HTTP codes derive from it.
- `Envelope::validate` fails on unknown operations, unknown `required_flags`
  and unknown non-namespaced fields; `x-`/`ext.` fields round-trip.
- `Receipt::is_locally_durable` is true only with a `local_commit` frontier.
- `clockwork::admit` is the only admission law; expansion never hard-admits.

**Error model.** Typed enums (`EnvelopeError`, `StoreError` in daemon); no
panics on caller input.

**Determinism class.** Pure functions of inputs; no clocks, I/O or randomness
except where a `Connection` is passed explicitly (`clockwork::links`).

**Cancellation.** Pure algorithms are synchronous. Rate-limiter operations take the caller's explicit `&asupersync::Cx`; mutex acquisition is fallible and cancellation propagates as `LockError`, not a successful admission or empty status.

**Unsafe.** `#![forbid(unsafe_code)]`.

**Feature flags.** None.

**Conformance tests.** `tests/contracts/protocol_types.rs`,
`clock_quorum.rs`, `baseline_fixtures.rs`, `conflict.rs`, `history.rs`.

**No-claim boundaries.** Type validity is not policy validity; admission is
relevance, not truth; digests are evidence under an algorithm, not identity.
