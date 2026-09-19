# cortex-kernel — contract

**Purpose / layer.** The embeddable Cortex brain: every memory semantic
(store path, CQR recall, boot compiler, schema and migrations, Reflex,
hook protocol, storage SPI) as in-process Rust. `cortex-daemon` is a
transport adapter over this crate; a Rust host path-deps it directly.

**Public types.** `CortexRuntime` (`open(&CortexPaths)`, `open_db(&Path)`,
`from_state`, `deposit`, `deposit_with_key`, `lens`, `boot`, `state`),
`LensInput`, `BootInput`, `DepositOutcome`, `CortexError`
(`Open | Rejected | Conflict | Recall | Internal | Lock`). `deposit` /
`cortex op commit` record caller `paths` / `cwd` / `cwd_path` /
`working_directory` / `thread` as explicit clock anchors. `process()` ToolResult
deposits stamp the same host cwd and session thread. `lens` / `orient` / `boot`
with those paths admit the matching project and drop a row whose explicit root
is a different repository; unscoped rows stay visible. Near-duplicate CQR
sentences in two repositories stay two facts: Jaccard merge only considers
rows whose explicit path is compatible with the incoming deposit. Unscoped
and path-scoped identities never merge with each other; two unscoped
near-duplicates still collapse. SessionStart boot
fallback compiles the CQR capsule with those roots, not an empty path list.
Boot packing omits identity and constraints capsules that do not
fit wholly; it does not cut those texts to save budget. Every module the daemon
used to own remains in this crate (`db`, `handlers::{operations,recall,store,…}`,
`state`, `compiler`, `reflex`, `store_spi`, …). Hosts and contracts import
`cortex_kernel` / `cortex_logic` directly. No axum / http / hyper type appears in
any public signature.

**Invariants.** `open()` never binds a socket, spawns a process or reads a
port; several handles may open the same database file; a deposit's Receipt
is identical whether it arrives through this crate, local CLI, MCP or a hook.

**Error model.** `CortexError` at the library boundary; engines below it
return `String` / `StoreError` and are mapped at `runtime/open.rs`.

**Determinism.** Deterministic given database state and inputs (same class
as `cortex-logic`); `now_iso` is the only clock read on the hot path.

**Cancellation.** Async operations require the host task's explicit
`&asupersync::Cx`: immediately after `&self` on runtime methods and first
on free functions. Read and write locks return cancellation/poison errors;
`CortexError::Lock` preserves the typed lock error at deposit/boot boundaries,
while recall/operations retain their string error boundary. Deferred telemetry
uses `lock_until` against the capability clock: only timeout queues work;
cancellation and poison propagate. Broadcast sends require the same capability.
Dropping a future before commit leaves no partial write (SAVEPOINT discipline
in `runtime/deposit.rs`); cancellation after a committed side effect does not
undo that effect. No detached contexts or runtime compatibility wrappers.

**Unsafe.** None.

**Feature flags.** `hybrid-search` / `vector-search` / `toon-payloads` /
`fsqlite-store` gate optional Dicklesworthstone-crate adapters
(`hybrid_search.rs`): frankensearch RRF fusion over FTS5 candidates,
hnswlib-rs ANN, TOON payload encoding, and a FrankenSQLite store SPI. All
default off; the SQLite FTS5 + Clock-Quorum path is the production default.
Hosts may also gate the whole crate (`--features cortex` in Outfit).

**Runtime capability boundary.** asupersync 0.4.10 supplies synchronization,
channels and time capabilities. The embedding host owns task/runtime lifetime
and supplies each operation's context; opening the synchronous SQLite state
does not create a runtime or task context. Adapters must forward capabilities
and handle fallible lock acquisition rather than treating cancellation as
successful work.

**Observation intake.** `runtime::observation` adds explicit local-principal
source registration, `observe`, normalized-JSONL `tail_observations`, source
cursor inspection, source enable/disable, and exact observation lookup. Source
roles are registered by the operator, never accepted from event bodies. Raw
observations remain attributed evidence, not verified facts or learned utility.
`commit` with `evidence: ["obs:<source_id>"]` is the only upgrade path: unknown,
disabled, retracted, or path-incompatible cites fail closed and roll the
deposit back. A cite attaches only to a durable decision id (a new row or a
merged survivor); a rejected duplicate with evidence fails closed. Cited
`obs:<id>` values are part of the deposit idempotency identity. A scoped commit may cite the unscoped `project` bucket; it cannot
cite a sibling repository, and a commit whose roots span sibling repositories
cannot cite a path-scoped observation from only one of them. Path identity
collapses `.` and `..` before that check: `repoa/../repob` is `repob`, not a
child of `repoa`. An unscoped commit cannot lift a path-scoped
observation. Operator-owned `promote-policy/1` (`db::promotion::set_policy` /
`load_policy`) can restrict who may run a typed promote rule, which
observation roles `commit` may cite, and which revocation triggers a
promoted revision records. Missing policy keeps the builtin defaults
(`preference` is `solo` / `user:*`; other rules are `*`; every cite role
except `delivery_only`; the hardcoded revocation strings). A stored
document that is not that schema, or that has the wrong shape, fails closed.
Unknown or `delivery_only` `commit_evidence.source_roles` fail at `set_policy`.
Cite links live in `decision_observation_evidence`, created with the rest of
schema init and copied by auto-repair.

Capture writes source bytes, a record/revision/head, source linkage, change
item, invalidation guards, receipt and existing outbox work in one SQLite
transaction. Tail cursors commit in that same transaction and advance only over
complete records. Identical independent events remain distinct; retries of one
source/generation/event key replay its receipt, while changed payloads conflict.
Receipts describe the configured SQLite acknowledgement profile, not a tested
power-loss guarantee. Known secret patterns are rejected before persistence;
this is not a comprehensive secret-scanning claim.

Limits: 2 MiB per submitted batch, 128 complete events per batch, and a
registered per-event text limit (64 KiB default, at most 2 MiB). New capture
honors hard outbox-debt backpressure. Source disable blocks intake and exact
read; scope pause blocks new intake but keeps existing exact reads available.
Scope stop and stale policy epochs fail closed. The normalized API accepts UTF-8 text without filesystem access.
`observe_file(cx, path)` and the existing indexers additionally read explicitly
registered `file:<canonical UTF-8 absolute path>` sources. They retain whole
files rather than previews, with a 1 MiB file ceiling and the registered text
limit, whichever is smaller. Read, authorization and SQL failures propagate.
Every file commits independently; `index_all` returns `Result<usize, String>`
and an error does not undo earlier files. Unchanged files replay their receipt.
The source role and scope still come only from registration; config metadata
and file prose cannot elevate authority. Legacy alias rows remain untouched.

File generations combine platform file identity metadata, modification time
and BLAKE3 content digest. Modification time supplies `observed_at`, not a
verified assertion timestamp. Metadata is rechecked after a bounded read;
this is not an atomic filesystem snapshot or a journal of intermediate edits.
Recursive custom-source enumeration does not follow symlinks and retains its
home-boundary check. Configured `truncate` previews no longer limit retained
source bytes. Source/record metadata is retained alongside the exact payload;
retained payload bytes are not a whole-database storage measurement.

**Inventory.** `inventory_sources`, `read_inventory`, `bootstrap_inventory`, `reconcile_inventory`
operate only on registered file keys for the trusted principal. Inventories are
persisted revisions (at most 4,096 entries); bootstrap uses a contiguous CAS
cursor and at most 128 sources / 16 MiB per invocation. Capture-before-checkpoint
failure is replay-safe. A blocked or changed entry never silently advances.
Restore epochs invalidate old progress. Ready means exact capture, not projection
or delivery completeness; an inventory is not a globally atomic file snapshot.
`reconcile_inventory` rechecks that sealed revision against current grants and
file metadata. Later registrations are counted, not appended.

**Retrieval and preparation.** `cycle::NeedSpec`, `query_observations`, `query_observations_for_paths`,
`recent_observations_for_paths`, `require_observation`,
`subscribe_observations` and `prepare_observations` implement scoped exact any-cue
joins over retained observations. A source `scope` may be a coarse label or a
project root (same path identity as CQR). `query` / `orient` with `paths` /
`cwd` pull compatible path-scoped sources plus the unscoped `project` bucket
when `observation_scope` is omitted; a default `project` pull does not leak
path-scoped sources. `CortexRuntime::lens` attaches the same attributed
`observations` section beside recall `results` (never mixed into `results`).
Orient with only those roots (or a task that is only
those path tokens) uses a recent-in-scope pull so cwd start can surface
attributed evidence without overlapping body cues. `cortex capture query`
accepts repeatable `--path` roots; `--scope` stays the extra/unscoped bucket
when paths are named, and remains required when no path is given. Capture maintains postings and reverse needs
inside its transaction. Catch-up projects at most 32 observations per slice;
8192 distinct cues per observation is the projection ceiling. At most 128 active
needs per principal, 32 query cues, 128 returned sources and 64 KiB payload.
Ready literal pulls read postings directly without temporary subscription writes
or a writer reservation. Schema initialization, projection catch-up and learned
routing can still require writes and return SQLite contention errors.
Reads revalidate consent, policy epochs, availability, heads and retractions;
no arbitrary exception logic, temporal query compiler or universal semantic
lifting is claimed. Unknown prose stays attributed evidence. Permission,
availability, projection debt and output limits qualify the response; incomplete
bundles are not automatically delivered. Exact references permit later hydration.
`rebuild_observation_projection` replaces only derived indexes; `retract_observation`
excludes active evidence without deleting exact bytes. `exclude_cues` drop optional
candidates. `require_observation` adds mandatory children; a missing or retracted
child yields `qualification_unavailable` rather than a prefix View.

Delivery presence is an explicit host assertion bound to principal, need,
context, fingerprint and restore epoch, expiring after five minutes. New content
or a new context requires a fresh payload. Delivery bytes are not model tokens.

**Host subsets.** `claude-visible-subset-v1` remains the legacy fixture adapter.
`CLAUDE_2_1_260_ADAPTER` (`claude-code-2.1.260-v1`) requires the exact 2.1.260
host-version pin. An isolated installed-host probe established that live
`prompt_id` matches historical `promptId`, NOT the historical message `uuid`.
Native user capture uses that stable prompt identity; an explicit conflicting
sidecar identity fails. Live final capture still requires a supplied final-message
UUID: a Stop event's prompt ID cannot identify multiple assistant completions.

Native Bash reports preserve the complete five-field reported response as JSON
(stdout, stderr, interrupted, isImage, noOutputExpected), using historical
`toolUseResult` for overlap. This preserves field values, not original JSON
whitespace/order or the shell's pre-host byte stream. Retrieval cues use reported
stdout/stderr, never JSON envelope names or boolean flags.

Known queue, budget, hook-context, ATIS latch, Bash request and last-prompt records
are non-evidence. Their byte offsets, lengths, kinds and BLAKE3 markers commit
with the raw cursor in `host_capture_metadata`; unknown/private shapes still
block advancement. No raw private metadata body is copied into evidence.
Operator-owned origin policy remains separate from payload origin claims.
`resolve_host_invocation` derives user, Bash, and file-tool identities from the
operator sidecar and host payload without model-generated memory commands. Own deliveries are excluded,
origin conflicts fail closed, and raw JSONL cursors commit atomically with capture
and metadata markers. A live payload `cwd` that looks like a project root uses
that path as the observation scope (same identity as CQR roots). Events without a
path stay in the grant's registered scope. A later capture of the same
generation/event key reuses the first source so catch-up cannot mint a second
row in the grant bucket. `capture_and_prepare_host` composes exact intake with
scoped needs; final/delivery-only events do not reinject. Capture may have committed
before a preparation error; retry the original identity.

Probe boundary: Claude Code 2.1.260 on macOS, isolated configuration, external
network denied, then deterministic loopback Messages responses to exercise Bash
and Stop. This validates observed native protocol shapes and context-channel
acceptance, NOT reader quality, real model-token savings, every tool shape or a
fully deployed Cortex/host installation.

**Associations.** Corpus routing is opt-in via `rebuild_associations`, bounded
to 512 eligible observations / 64 KiB each and 32 lexical features per source.
Current authorization and source lineage are revalidated; copied text and connected
lineages do not multiply support. Agent/tool derivatives and own deliveries are
not independent support. Learned candidates remain separately labeled and cannot
alter source authority or hard gates. Explicit named usefulness assessments are
idempotent, reversible and bounded in scoring effect; later tool success does not
implicitly earn causal credit. `reset_associations` disables refresh until rebuild.
No held-out benefit or neural/controller implementation is claimed.

**Assemblies.** `put_assembly` records exact membership over existing revision
rows, including contradictions. Factoring is typed JSON (`raw` or
`template_residual`); expand reconstructs the original bodies. Learning events
are attributed, idempotent, retractable, and source-erasable. Cue routes stay
disabled until `rebuild_assembly_routes`. `explain_assembly_routes` cites cues
and training units; ranking is scoped utility, not a truth score. A
`NeedSpec` with `learned: true` may add separately labeled
`learned_assembly_route` / `assembly_exception` candidates only after that
rebuild, still inside current grants and evidence closure. `compile_assemblies`
is the evidence-closed View for those routes: `orient` / `query` attach an
`assemblies` section (never Cards), `expand` hydrates `asm:` / `rev:`, and
hooks inject the extractive brief. `compile_assemblies_for_paths` joins
caller roots the same way as observation pulls. A missing required exception yields
`qualification_unavailable` with no prefix claim. Host presence may omit
payloads already attested in the current invocation. Default query and
routes-off reads stay exact observation / CQR until rebuild. `lens` attaches
the same `assemblies` section beside `results`. SessionStart boot fallback
compiles the CQR capsule and assemblies from caller `cwd` rather than an empty
path list or only the `project` bucket. File inject (`boot_for_paths` /
`cortex prompt-inject --path`) uses the same roots. No HDC or
controller is claimed.

**Host entry.** Live host events use `cortex hook <kind>`. `resolve_host_invocation`
turns `CORTEX_CAPTURE` or `$CORTEX_HOME/capture.json` plus the host payload into
a `HostInvocation`, or returns `None` so the command stays silent. CQR hook
protocol remains `process()`, used by `cortex hook-boot` (SessionStart orient)
and by hosts that call it directly. There is no second command name and no
second environment name for that sidecar. `cortex setup` writes the sidecar
file once; an existing operator file is left alone.
`cortex capture host-cycle` remains the explicit-flag operator path.

**Identity sidecar.** The runtime holds no RefZero handle; the `refzero`
module opens the session-owned `<brain>.refzero.sqlite` sidecar next to
the brain DB on demand and interns deposit/host-capture bytes there
(best-effort, never failing the store; hook handles are cached per sidecar
path, harness handles from `open_sidecar` are never shared). It
exports/imports the exact
cross-product `RecallEnvelope` shape and `z://blob/` + `#B`/`#L` grammar;
import mints a fresh loc with no Edit grant. Digest spellings in identity
slots fail closed as `not_a_loc`. Hashing is BLAKE3-only; migration 025
renames the host marker column, and legacy 16-hex seals dual-read as
unverifiable (cold rows re-seal when the length matches, ledger rows
conflict).

**Conformance.** `tests/contracts/observation_capture.rs`,
`observation_inventory.rs`, `observation_cycle.rs`, `host_capture.rs`,
`observation_associations.rs`, `assembly.rs`, `tests/contracts/kernel_embed.rs`
(`kernel_opens_deposits_and_lenses_without_a_server_or_a_port`), and
`tests/contracts/refzero_{identity,interop}.rs` (identity store, importer,
shared `z://blob/` vectors, envelope transfer, migration 025). Contracts
import `cortex_kernel` / `cortex_logic` for brain types and `cortex_daemon`
only for MCP, health presentation, and other process-edge surfaces.

**No-claim boundaries.** Not a stable ABI across languages; not thread-safe
beyond what `RuntimeState` (asupersync mutexes) provides; `boot()` compiles the
legacy capsule, the operations surface is `handlers::operations::dispatch`.
