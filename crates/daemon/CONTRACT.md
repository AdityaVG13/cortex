# cortex-daemon -- contract

**Purpose / layer.** Local CLI, MCP stdio and host hooks over `cortex-kernel`.
The binary retains the name `cortex`; the crate name is historical. There is
no HTTP listener, TLS server, proxy, remote target, or OS service controller.
The kernel library and storage SPI are re-exported for Rust callers.

**Entry points.** `cortex mcp`, `cortex op`, `cortex boot`, `cortex maintain`,
local diagnostics/backup commands, and in-process hooks. `cortex serve` is an
optional headless worker: it polls SQLite data version, drains at most 32
outbox jobs per tick, and holds a home-scoped worker lock. It is not needed
for CLI or MCP writes and does not bind a port.

**Invariants.**
- CLI, native MCP and hooks share kernel dispatch and durable SQLite writes.
- Local process/file access is the authority boundary. Caller ownership is
  supplied by the adapter, not trusted from memory text or request fields.
- JSON-RPC IDs, notifications and errors retain their protocol shape; domain
  statuses live inside results. No HTTP status/header contract remains.
- Idempotency, revision/head conflicts, retention and durability are governed
  by the kernel contract, not by transport retries or an external write buffer.

**Cancellation.** Asupersync owns runtime, locks, channels and timers. Adapters
forward the caller's `&Cx` to asynchronous kernel operations and propagate
fallible lock acquisition. A cancelled waiter does not perform a write;
cancellation cannot undo a previously committed SQLite batch. Synchronous
SQLite calls are not interruptible in the middle of a statement. MCP stdin
is bounded to 2 MiB per line and read synchronously by its dedicated host
process; no cancellation-latency claim is made while waiting for stdin.

**Unsafe boundary.** Path environment initialization occurs only before
runtime worker creation. Platform file permissions use the kernel's audited
OS boundary. The crate denies undocumented unsafe operations; it does not
claim to forbid every unsafe block.

**Conformance.** `tests/contracts/kernel_embed.rs`, `native_entry.rs`,
`operations.rs`, `admin_acl.rs`, `wire_error_contract.rs`,
`crash_durability.rs`, and `cli_goldens.rs`.

**No-claim boundaries.** No remote/multi-host service, HTTP SDK compatibility,
legacy Control Center compatibility, stable foreign ABI or exact tokenizer
accounting. Durability claims are those recorded in each receipt. Old tagged
releases retain the removed desktop/server product; changing this repository
does not migrate external installations or existing databases.
