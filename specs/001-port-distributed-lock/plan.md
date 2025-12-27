# Implementation Plan: Port DistributedLock to Rust

**Branch**: `001-port-distributed-lock` | **Date**: 2025-12-27 | **Spec**: [spec.md](./spec.md)  
**Input**: Feature specification from `/specs/001-port-distributed-lock/spec.md`

## Summary

Port the DistributedLock C# library to Rust, providing distributed synchronization primitives (mutex locks, reader-writer locks, semaphores) with multiple backend support (PostgreSQL, Redis, file system). The Rust implementation will follow idiomatic patterns (traits, RAII, async/await) while preserving the proven architecture of the C# library.

**Key Technical Approach**:
- Trait-based abstraction (`DistributedLock`, `LockHandle`, `LockProvider`)
- Async-first design with tokio runtime
- Workspace structure with per-backend crates
- Connection pooling via `deadpool` family

## Technical Context

**Language/Version**: Rust 1.75+ (stable, 2024 edition)  
**Primary Dependencies**:
- `tokio` 1.x (async runtime)
- `tokio-postgres` 0.7.x + `deadpool-postgres` 0.14.x (Postgres backend)
- `fred` 9.x (Redis backend)
- `fd-lock` 4.x (file locking)
- `thiserror` 1.x (error types)
- `tracing` 0.1.x (structured logging)

**Storage**: PostgreSQL (advisory locks), Redis (key expiry), File system (OS locks)  
**Testing**: `cargo test`, `testcontainers` for integration tests  
**Target Platform**: Linux, macOS, Windows (cross-platform)  
**Project Type**: Rust workspace (multi-crate library)  
**Performance Goals**: Lock acquisition <10ms overhead beyond backend latency  
**Constraints**: No blocking in async context; RAII-based resource cleanup  
**Scale/Scope**: Production-ready library with comprehensive test coverage

## Constitution Check

*GATE: All principles verified ✅*

| Principle | Status | Evidence |
|-----------|--------|----------|
| I. MVP-First Delivery | ✅ Pass | P1 user stories (mutex, async) implemented first; P3 deferred |
| II. Code Quality Standards | ✅ Pass | Public API docs required; clippy/fmt in CI |
| III. Testing Discipline | ✅ Pass | Integration tests for all public APIs; testcontainers for DBs |
| IV. API Consistency | ✅ Pass | All lock types share same trait signatures |
| V. Performance by Design | ✅ Pass | Async-first; connection pooling; benchmarks planned |

**Simplicity Constraints Check**:
- ✅ No speculative features (only porting proven C# functionality)
- ✅ No premature abstraction (traits mirror C# interfaces with real usage)
- ✅ Maximum 3 layers (Lock → Backend → Driver)

## Project Structure

### Documentation (this feature)

```text
specs/001-port-distributed-lock/
├── plan.md              # This file
├── spec.md              # Feature specification
├── research.md          # C# analysis + Rust ecosystem research
├── data-model.md        # Rust type definitions
├── quickstart.md        # Usage examples
├── contracts/           # Trait definitions (pseudo-Rust)
│   ├── core-traits.rs
│   ├── postgres-backend.rs
│   ├── redis-backend.rs
│   └── file-backend.rs
└── checklists/
    └── requirements.md  # Spec quality checklist
```

### Source Code (repository root)

```text
Cargo.toml                          # Workspace manifest

crates/
├── distributed-lock-core/          # Core traits and common utilities
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                  # Public re-exports
│       ├── traits.rs               # DistributedLock, LockHandle, etc.
│       ├── error.rs                # LockError enum
│       ├── timeout.rs              # TimeoutValue helper
│       └── prelude.rs              # Convenient imports
│
├── distributed-lock-postgres/      # PostgreSQL advisory lock backend
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── provider.rs             # PostgresLockProvider
│       ├── lock.rs                 # PostgresDistributedLock
│       ├── handle.rs               # PostgresLockHandle
│       ├── key.rs                  # PostgresAdvisoryLockKey
│       ├── rw_lock.rs              # PostgresDistributedReaderWriterLock
│       └── connection.rs           # Connection/pool management
│
├── distributed-lock-redis/         # Redis backend with RedLock
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── provider.rs             # RedisLockProvider
│       ├── lock.rs                 # RedisDistributedLock
│       ├── handle.rs               # RedisLockHandle + lease extension
│       ├── redlock/                # RedLock algorithm
│       │   ├── mod.rs
│       │   ├── acquire.rs
│       │   ├── extend.rs
│       │   └── release.rs
│       ├── rw_lock.rs              # RedisDistributedReaderWriterLock
│       └── semaphore.rs            # RedisDistributedSemaphore
│
├── distributed-lock-file/          # File system lock backend
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── provider.rs             # FileLockProvider
│       ├── lock.rs                 # FileDistributedLock
│       ├── handle.rs               # FileLockHandle
│       └── name.rs                 # File name escaping
│
└── distributed-lock/               # Meta-crate re-exporting all
    ├── Cargo.toml
    └── src/
        └── lib.rs                  # Re-exports from all backends

tests/                              # Integration tests
├── postgres_tests.rs
├── redis_tests.rs
├── file_tests.rs
└── common/
    └── mod.rs                      # Shared test utilities
```

**Structure Decision**: Cargo workspace with separate crates per backend. This mirrors the C# NuGet package structure, allowing users to depend only on the backends they need. The meta-crate `distributed-lock` provides a single-dependency option for users who want everything.

## Implementation Phases

### Phase 1: Core + File Backend (MVP) - P1 Stories

1. **distributed-lock-core**: Traits, errors, timeout helpers
2. **distributed-lock-file**: Simplest backend for testing patterns
3. Integration tests proving mutex semantics work

**Deliverable**: Working file-based locks with async/sync APIs

### Phase 2: PostgreSQL Backend - P1/P2 Stories

1. **distributed-lock-postgres**: Advisory lock implementation
2. Connection pooling with multiplexing
3. Reader-writer lock support
4. Handle lost detection via connection monitoring

**Deliverable**: Production-ready Postgres distributed locks

### Phase 3: Redis Backend - P2/P3 Stories

1. **distributed-lock-redis**: Single-server locks
2. RedLock algorithm for multi-server
3. Lease extension background task
4. Semaphore implementation

**Deliverable**: Full Redis backend with RedLock support

### Phase 4: Polish - All Stories

1. Meta-crate and documentation
2. Benchmarks and optimization
3. CI/CD pipeline
4. Crates.io publishing preparation

**Deliverable**: Published crate ready for production use

## Dependencies Between Components

```
distributed-lock-core
    ↑
    ├── distributed-lock-postgres
    ├── distributed-lock-redis  
    └── distributed-lock-file
              ↑
              └── distributed-lock (meta-crate)
```

Each backend crate depends only on `core`. The meta-crate depends on all backends.

## Complexity Tracking

No constitution violations to justify. The project follows standard patterns:

- Workspace structure = idiomatic for multi-crate Rust libraries
- Trait hierarchy = mirrors proven C# interface design
- 3 backends = each is a distinct, self-contained implementation

---

## Generated Artifacts

| Artifact | Path | Description |
|----------|------|-------------|
| Research | [research.md](./research.md) | C# architecture analysis, Rust crate selection |
| Data Model | [data-model.md](./data-model.md) | Rust type definitions and traits |
| Contracts | [contracts/](./contracts/) | Trait and API contract definitions |
| Quickstart | [quickstart.md](./quickstart.md) | Usage examples for all backends |

## Next Steps

Run `/speckit.tasks` to generate the detailed task breakdown for implementation.
