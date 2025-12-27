# Research: Port DistributedLock to Rust

**Feature Branch**: `001-port-distributed-lock`  
**Date**: 2025-12-27

## Executive Summary

This document captures research findings for porting the DistributedLock C# library to Rust. The C# library provides a mature, battle-tested implementation of distributed synchronization primitives with multiple backend support. Our Rust port will preserve the core abstractions while adapting to Rust's ownership model and async ecosystem.

---

## C# Library Architecture Analysis

### Core Design Patterns

**Decision**: Adopt a trait-based architecture mirroring C#'s interface hierarchy.

**Rationale**: The C# library uses a clean interface-based design that maps naturally to Rust traits:
- `IDistributedLock` → `DistributedLock` trait
- `IDistributedSynchronizationHandle` → `LockHandle` trait  
- `IDistributedLockProvider` → `LockProvider` trait

**Alternatives considered**:
- Direct struct implementations without traits → Rejected; loses backend abstraction
- Enum-based dispatch → Rejected; less extensible for user-defined backends

### Key Internal Patterns

| C# Pattern | Rust Equivalent | Notes |
|------------|-----------------|-------|
| `IAsyncDisposable` + `IDisposable` | `Drop` trait + async `release()` | RAII via Drop, explicit async release optional |
| `CancellationToken` | `tokio_util::sync::CancellationToken` | Standard async cancellation |
| `ValueTask<T>` | `impl Future<Output = T>` | Zero-cost async |
| `TimeoutValue` struct | `Option<Duration>` | None = infinite timeout |
| `SyncViaAsync` helper | Block with `tokio::runtime::Handle::block_on` | For sync API on async core |
| Generic `THandle` | Associated types on traits | Type-safe handle per backend |

### Synchronization Primitives

1. **Mutex Lock** (`IDistributedLock`)
   - Acquire/TryAcquire with timeout
   - Async variants
   - Non-reentrant

2. **Reader-Writer Lock** (`IDistributedReaderWriterLock`)
   - Multiple concurrent readers OR single writer
   - Writer priority to prevent starvation
   - Upgradeable variant exists but complex (defer to P3+)

3. **Semaphore** (`IDistributedSemaphore`)
   - N concurrent tickets
   - Same acquire pattern as locks

---

## Backend Implementation Analysis

### PostgreSQL Advisory Locks

**Decision**: Use `tokio-postgres` with `deadpool-postgres` connection pool.

**Rationale**: 
- `tokio-postgres` is the most mature async Postgres driver
- `deadpool-postgres` provides production-ready connection pooling
- Advisory lock functions (`pg_advisory_lock`, `pg_try_advisory_lock`) are well-documented

**Key Implementation Details from C#**:
- Lock keys: 64-bit integer OR pair of 32-bit integers (different key spaces)
- Name hashing: SHA-1 hash truncated to 64 bits for string names
- ASCII encoding: 9-char ASCII strings map to unique 64-bit keys
- Transaction-scoped locks: `pg_advisory_xact_lock` (held until transaction ends)
- Connection-scoped locks: `pg_advisory_lock` (held until explicit unlock or disconnect)
- Timeout: Set via `SET LOCAL lock_timeout = N`

**SQL Functions**:
```sql
-- Exclusive lock (blocking)
SELECT pg_catalog.pg_advisory_lock(@key)

-- Exclusive lock (non-blocking)  
SELECT pg_catalog.pg_try_advisory_lock(@key)

-- Shared lock (for readers)
SELECT pg_catalog.pg_advisory_lock_shared(@key)

-- Release
SELECT pg_catalog.pg_advisory_unlock(@key)
```

**Alternatives considered**:
- `sqlx` → Viable alternative, slightly higher level; may use if type-safety benefits outweigh complexity

### Redis Locks (RedLock Algorithm)

**Decision**: Use `fred` crate for Redis client with custom RedLock implementation.

**Rationale**:
- `fred` is modern, async-first, and actively maintained
- RedLock algorithm is well-specified; custom implementation preferred for control
- `deadpool-redis` available if pooling needed

**Key Implementation Details from C#**:
- Lock: `SET key value NX PX milliseconds` (NX = not exists, PX = expiry)
- Extend: Lua script comparing value before `PEXPIRE`
- Release: Lua script comparing value before `DEL`
- Lock ID: Random value (GUID) to ensure only holder can release
- Expiry: Default 30s with extension cadence of expiry/3
- Multi-server: Acquire on majority (N/2 + 1) of servers

**Lua Scripts**:
```lua
-- Extend (from C#)
if redis.call('get', KEYS[1]) == ARGV[1] then
    return redis.call('pexpire', KEYS[1], ARGV[2])
end
return 0

-- Release (from C#)
if redis.call('get', KEYS[1]) == ARGV[1] then
    return redis.call('del', KEYS[1])
end
return 0
```

**Alternatives considered**:
- `redis` crate → Older, less ergonomic async; `fred` preferred
- Existing `redlock` crate → Evaluate for reuse vs implementing fresh

### File System Locks

**Decision**: Use `fd-lock` crate with `fs2` as fallback.

**Rationale**:
- `fd-lock` provides cross-platform advisory file locking
- Uses OS-native mechanisms (flock on Unix, LockFileEx on Windows)
- Simpler than C# implementation which uses FileStream with FileShare.None

**Key Implementation Details from C#**:
- Lock file: `FileStream` with `FileShare.None` + `FileOptions.DeleteOnClose`
- Busy wait: Poll with 50ms-1s random sleep between attempts
- Directory creation: Automatic, with retry for race conditions

**Rust Approach**:
```rust
// Using fd-lock
let file = File::create(path)?;
let mut lock = fd_lock::RwLock::new(file);
let guard = lock.try_write()?; // Non-blocking
// Guard dropped → lock released
```

**Alternatives considered**:
- `fs2::FileExt::lock_exclusive()` → Good option, evaluate API ergonomics
- Raw libc calls → Too low-level, not cross-platform

---

## Rust Ecosystem Dependencies

### Async Runtime

**Decision**: Primary support for `tokio`; runtime-agnostic where possible.

**Rationale**: 
- Tokio is the de facto standard for async Rust
- All chosen DB drivers are tokio-based
- Can use `async-trait` crate for trait async methods (until RPITIT stabilizes)

### Connection Pooling

**Decision**: Use `deadpool` family for connection pooling.

**Rationale**:
- `deadpool-postgres` and `deadpool-redis` are well-maintained
- Consistent API across backends
- Supports metrics and health checks

### Error Handling

**Decision**: Use `thiserror` for error types; expose `anyhow`-compatible errors.

**Rationale**:
- `thiserror` generates clean error types with minimal boilerplate
- Each backend can have specific error variants
- Users can use `?` operator seamlessly

### Handle Lost Detection

**Decision**: Use `tokio::sync::watch` channel for handle lost notification.

**Rationale**:
- Similar to C#'s `CancellationToken` pattern
- Zero-cost when not polled
- Can be triggered by connection monitoring or lease expiry

---

## Crate Structure

**Decision**: Workspace with separate crates per backend.

**Rationale**: Matches C# NuGet package structure; users only depend on backends they need.

```
distributed-lock/
├── Cargo.toml (workspace)
├── distributed-lock-core/        # Traits, errors, common utilities
│   └── Cargo.toml
├── distributed-lock-postgres/    # PostgreSQL advisory locks
│   └── Cargo.toml
├── distributed-lock-redis/       # Redis/RedLock implementation
│   └── Cargo.toml
├── distributed-lock-file/        # File system locks
│   └── Cargo.toml
└── distributed-lock/             # Meta-crate re-exporting all
    └── Cargo.toml
```

**Alternatives considered**:
- Single crate with features → Rejected; feature flags get complex with many backends
- Single crate including all → Rejected; forces unnecessary dependencies

---

## API Design Decisions

### Trait Hierarchy

```rust
/// Core lock trait - all backends implement this
pub trait DistributedLock: Send + Sync {
    type Handle: LockHandle;
    
    fn name(&self) -> &str;
    
    async fn acquire(&self, timeout: Option<Duration>) -> Result<Self::Handle, LockError>;
    
    async fn try_acquire(&self) -> Result<Option<Self::Handle>, LockError>;
}

/// Handle returned when lock is acquired
pub trait LockHandle: Send + Sync {
    /// Check if lock was lost (e.g., connection died)
    fn lost_token(&self) -> &tokio::sync::watch::Receiver<bool>;
    
    /// Explicitly release (also happens on Drop)
    async fn release(self) -> Result<(), LockError>;
}

/// Factory for creating locks by name
pub trait LockProvider: Send + Sync {
    type Lock: DistributedLock;
    
    fn create_lock(&self, name: &str) -> Self::Lock;
}
```

### Synchronous API

**Decision**: Provide sync wrappers that block on async core.

**Rationale**: Some users may not use async; C# library supports both.

```rust
impl<L: DistributedLock> SyncDistributedLock for L {
    fn acquire_blocking(&self, timeout: Option<Duration>) -> Result<Self::Handle, LockError> {
        tokio::runtime::Handle::current().block_on(self.acquire(timeout))
    }
}
```

### Timeout Semantics

| Method | Default Timeout | Behavior on Timeout |
|--------|-----------------|---------------------|
| `acquire(None)` | Infinite | Wait forever |
| `acquire(Some(d))` | d | Return `Err(LockError::Timeout)` |
| `try_acquire()` | Zero | Return `Ok(None)` if unavailable |

---

## Testing Strategy

### Unit Tests
- Test name hashing/escaping logic
- Test timeout value handling
- Test error type conversions

### Integration Tests (require infrastructure)
- Postgres: Use `testcontainers` to spin up Postgres
- Redis: Use `testcontainers` for Redis
- File: Use temp directories

### Property-Based Tests
- Fuzz lock name generation
- Verify mutual exclusion with concurrent tasks

### Benchmarks
- Compare acquire/release latency across backends
- Measure overhead vs direct DB calls

---

## Open Questions (Resolved)

| Question | Resolution |
|----------|------------|
| Which async runtime? | Tokio (primary), with async-trait for trait methods |
| How to handle sync API? | Block on async using `Handle::block_on` |
| Crate structure? | Workspace with per-backend crates |
| Connection pooling? | deadpool family |
| Handle lost detection? | watch channel, triggered by connection monitor |

---

## Dependencies Summary

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | 1.x | Async runtime |
| `async-trait` | 0.1.x | Async methods in traits |
| `thiserror` | 1.x | Error type derivation |
| `tokio-postgres` | 0.7.x | Postgres driver |
| `deadpool-postgres` | 0.14.x | Postgres connection pool |
| `fred` | 9.x | Redis client |
| `fd-lock` | 4.x | File locking |
| `sha2` | 0.10.x | Name hashing (for Postgres keys) |
| `rand` | 0.8.x | Random lock IDs (for Redis) |
| `tracing` | 0.1.x | Structured logging |

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Tokio runtime mismatch | Low | High | Document tokio requirement clearly |
| Connection pool exhaustion | Medium | Medium | Configurable pool size, good defaults |
| RedLock timing issues | Medium | High | Implement clock drift tolerance per spec |
| File lock portability | Low | Medium | Test on Linux, macOS, Windows |
| Async trait overhead | Low | Low | Minimal; may use RPITIT when stable |
