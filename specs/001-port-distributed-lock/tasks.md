# Implementation Tasks: Port DistributedLock to Rust

**Feature**: Port DistributedLock C# library to Rust  
**Branch**: `001-port-distributed-lock`  
**Generated**: 2025-12-27

## Overview

This document breaks down the implementation into dependency-ordered phases, organized by user story priority. Each phase is independently testable and can be delivered incrementally.

**Total Tasks**: 111  
**MVP Scope**: Phases 1-4 (Core + File Backend) - 41 tasks  
**Full Scope**: All phases - 111 tasks

---

## Dependencies

### User Story Completion Order

```
Phase 1: Setup
    ↓
Phase 2: Foundational (Core Traits)
    ↓
Phase 3: US1 - Exclusive Lock (P1)
    ↓
Phase 4: US2 - Async Lock Acquisition (P1)
    ↓
Phase 5: US3 - Backend-Agnostic Provider (P2)
    ↓
Phase 6: US4 - Reader-Writer Lock (P2)
    ↓
Phase 7: US6 - Postgres Backend (P2)
    ↓
Phase 8: US5 - Semaphore (P3)
    ↓
Phase 9: US7 - Redis Backend (P3)
    ↓
Phase 10: US8 - File Backend (P3)
    ↓
Phase 11: Polish & Cross-Cutting
```

### Parallel Execution Opportunities

- **Phase 2**: Core trait definitions can be implemented in parallel (T009-T018)
- **Phase 3**: File backend implementation tasks (T023-T032) can be parallelized
- **Phase 7**: Postgres backend components (T055-T071) can be parallelized
- **Phase 9**: Redis backend components (T080-T095) can be parallelized
- **Phase 10**: File backend components (T096-T098) can be parallelized

---

## Phase 1: Setup

**Goal**: Initialize workspace structure and project configuration.

**Independent Test**: Workspace compiles with `cargo check` and all crates are discoverable.

### Tasks

- [X] T001 Create Cargo workspace manifest at Cargo.toml
- [X] T002 Create distributed-lock-core crate structure at crates/distributed-lock-core/Cargo.toml
- [X] T003 Create distributed-lock-postgres crate structure at crates/distributed-lock-postgres/Cargo.toml
- [X] T004 Create distributed-lock-redis crate structure at crates/distributed-lock-redis/Cargo.toml
- [X] T005 Create distributed-lock-file crate structure at crates/distributed-lock-file/Cargo.toml
- [X] T006 Create distributed-lock meta-crate structure at crates/distributed-lock/Cargo.toml
- [X] T007 Create tests directory structure at tests/common/mod.rs
- [X] T008 Add workspace dependencies (tokio, thiserror, tracing) to workspace Cargo.toml

---

## Phase 2: Foundational

**Goal**: Implement core traits, error types, and timeout helpers that all backends depend on.

**Independent Test**: Core crate compiles and exports all public traits/types. Error types can be constructed and converted.

### Tasks

- [X] T009 [P] Define LockError enum in crates/distributed-lock-core/src/error.rs
- [X] T010 [P] Define TimeoutValue helper in crates/distributed-lock-core/src/timeout.rs
- [X] T011 [P] Define LockHandle trait in crates/distributed-lock-core/src/traits.rs
- [X] T012 [P] Define DistributedLock trait in crates/distributed-lock-core/src/traits.rs
- [X] T013 [P] Define DistributedReaderWriterLock trait in crates/distributed-lock-core/src/traits.rs
- [X] T014 [P] Define DistributedSemaphore trait in crates/distributed-lock-core/src/traits.rs
- [X] T015 [P] Define LockProvider trait in crates/distributed-lock-core/src/traits.rs
- [X] T016 [P] Define ReaderWriterLockProvider trait in crates/distributed-lock-core/src/traits.rs
- [X] T017 [P] Define SemaphoreProvider trait in crates/distributed-lock-core/src/traits.rs
- [X] T018 [P] Define LockProviderExt extension trait in crates/distributed-lock-core/src/traits.rs
- [X] T019 Create prelude module in crates/distributed-lock-core/src/prelude.rs
- [X] T020 Create lib.rs with public re-exports in crates/distributed-lock-core/src/lib.rs
- [X] T021 Add thiserror dependency to distributed-lock-core/Cargo.toml
- [X] T022 Add tokio dependency to distributed-lock-core/Cargo.toml

---

## Phase 3: User Story 1 - Exclusive Lock (P1)

**Goal**: Implement basic exclusive mutex lock functionality with blocking acquire/try_acquire.

**Independent Test**: Can create a lock, acquire it, verify exclusivity across processes, and release it. Delivers immediate value for protecting critical sections.

**Test Criteria**: 
- Lock acquisition blocks other processes
- Lock release allows waiting processes to acquire
- Try-acquire returns None when lock is held
- Lock is released on handle drop

### Tasks

- [X] T023 [US1] Implement FileDistributedLock struct in crates/distributed-lock-file/src/lock.rs
- [X] T024 [US1] Implement FileLockHandle struct in crates/distributed-lock-file/src/handle.rs
- [X] T025 [US1] Implement FileLockProvider struct in crates/distributed-lock-file/src/provider.rs
- [X] T026 [US1] Implement FileLockProviderBuilder in crates/distributed-lock-file/src/provider.rs
- [X] T027 [US1] Implement file name validation/escaping in crates/distributed-lock-file/src/name.rs
- [X] T028 [US1] Add fd-lock dependency to distributed-lock-file/Cargo.toml
- [X] T029 [US1] Add distributed-lock-core dependency to distributed-lock-file/Cargo.toml
- [X] T030 [US1] Implement blocking acquire for FileDistributedLock in crates/distributed-lock-file/src/lock.rs
- [X] T031 [US1] Implement try_acquire for FileDistributedLock in crates/distributed-lock-file/src/lock.rs
- [X] T032 [US1] Implement release for FileLockHandle in crates/distributed-lock-file/src/handle.rs
- [X] T033 [US1] Implement Drop for FileLockHandle in crates/distributed-lock-file/src/handle.rs
- [X] T034 [US1] Create integration test for exclusive lock semantics in tests/file_tests.rs
- [X] T035 [US1] Create lib.rs with public re-exports in crates/distributed-lock-file/src/lib.rs

---

## Phase 4: User Story 2 - Async Lock Acquisition (P1)

**Goal**: Add async variants of acquire operations compatible with Rust's async/await.

**Independent Test**: Async acquire completes via await without blocking threads. Multiple async tasks waiting are served in FIFO order.

**Test Criteria**:
- Async acquire yields without blocking runtime
- Multiple async tasks can wait concurrently
- Cancellation tokens work correctly
- FIFO ordering is maintained

### Tasks

- [X] T036 [US2] Implement async acquire for FileDistributedLock in crates/distributed-lock-file/src/lock.rs
- [X] T037 [US2] Implement async try_acquire for FileDistributedLock in crates/distributed-lock-file/src/lock.rs
- [X] T038 [US2] Implement async release for FileLockHandle in crates/distributed-lock-file/src/handle.rs
- [X] T039 [US2] Add tokio dependency to distributed-lock-file/Cargo.toml
- [X] T040 [US2] Create async integration test in tests/file_tests.rs
- [X] T041 [US2] Add cancellation token support to acquire methods in crates/distributed-lock-core/src/traits.rs (Note: Cancellation handled via future dropping/tokio::select!)

---

## Phase 5: User Story 3 - Backend-Agnostic Provider (P2)

**Goal**: Implement provider abstraction enabling dependency injection and backend swapping.

**Independent Test**: Can create a mock provider implementing LockProvider trait and use it identically to real backends.

**Test Criteria**:
- Provider trait creates locks by name
- Application code works with any provider implementation
- Backend can be swapped without code changes

### Tasks

- [X] T042 [US3] Implement LockProvider for FileLockProvider in crates/distributed-lock-file/src/provider.rs
- [X] T043 [US3] Create mock provider for testing in tests/common/mock_provider.rs
- [X] T044 [US3] Add provider extension methods to LockProviderExt in crates/distributed-lock-core/src/traits.rs
- [X] T045 [US3] Create provider abstraction test in tests/provider_tests.rs

---

## Phase 6: User Story 4 - Reader-Writer Lock (P2)

**Goal**: Implement reader-writer locks allowing multiple concurrent readers OR single exclusive writer.

**Independent Test**: Multiple processes can acquire read locks concurrently. Write lock blocks all readers. Writers get priority to prevent starvation.

**Test Criteria**:
- Multiple readers can hold lock simultaneously
- Write lock blocks all readers
- Readers block when writer holds lock
- Writer priority prevents starvation

### Tasks

- [X] T046 [US4] Implement PostgresDistributedReaderWriterLock struct in crates/distributed-lock-postgres/src/rw_lock.rs
- [X] T047 [US4] Implement PostgresReadLockHandle struct in crates/distributed-lock-postgres/src/rw_lock.rs
- [X] T048 [US4] Implement PostgresWriteLockHandle struct in crates/distributed-lock-postgres/src/rw_lock.rs
- [X] T049 [US4] Implement acquire_read for PostgresDistributedReaderWriterLock in crates/distributed-lock-postgres/src/rw_lock.rs
- [X] T050 [US4] Implement acquire_write for PostgresDistributedReaderWriterLock in crates/distributed-lock-postgres/src/rw_lock.rs
- [X] T051 [US4] Implement try_acquire_read for PostgresDistributedReaderWriterLock in crates/distributed-lock-postgres/src/rw_lock.rs
- [X] T052 [US4] Implement try_acquire_write for PostgresDistributedReaderWriterLock in crates/distributed-lock-postgres/src/rw_lock.rs
- [X] T053 [US4] Implement ReaderWriterLockProvider for PostgresLockProvider in crates/distributed-lock-postgres/src/provider.rs
- [X] T054 [US4] Create reader-writer lock integration test in tests/postgres_tests.rs

---

## Phase 7: User Story 6 - Postgres Backend (P2)

**Goal**: Implement PostgreSQL advisory lock backend with connection pooling and handle lost detection.

**Independent Test**: Can acquire Postgres advisory locks, verify exclusivity across connections, and detect connection loss.

**Test Criteria**:
- Postgres advisory locks are exclusive across connections
- Lock keys are correctly encoded from names
- Connection loss is detected and reported
- Connection pooling works correctly

### Tasks

- [X] T055 [US6] Implement PostgresAdvisoryLockKey enum in crates/distributed-lock-postgres/src/key.rs
- [X] T056 [US6] Implement from_name for PostgresAdvisoryLockKey in crates/distributed-lock-postgres/src/key.rs
- [X] T057 [US6] Implement PostgresLockProvider struct in crates/distributed-lock-postgres/src/provider.rs
- [X] T058 [US6] Implement PostgresLockProviderBuilder in crates/distributed-lock-postgres/src/provider.rs
- [X] T059 [US6] Implement PostgresDistributedLock struct in crates/distributed-lock-postgres/src/lock.rs
- [X] T060 [US6] Implement PostgresLockHandle struct in crates/distributed-lock-postgres/src/handle.rs
- [X] T061 [US6] Implement connection pool management in crates/distributed-lock-postgres/src/connection.rs
- [X] T062 [US6] Implement acquire for PostgresDistributedLock using pg_advisory_lock in crates/distributed-lock-postgres/src/lock.rs
- [X] T063 [US6] Implement try_acquire for PostgresDistributedLock using pg_try_advisory_lock in crates/distributed-lock-postgres/src/lock.rs
- [X] T064 [US6] Implement release for PostgresLockHandle using pg_advisory_unlock in crates/distributed-lock-postgres/src/handle.rs
- [X] T065 [US6] Implement handle lost detection via connection monitoring in crates/distributed-lock-postgres/src/handle.rs
- [X] T066 [US6] Add tokio-postgres dependency to distributed-lock-postgres/Cargo.toml
- [X] T067 [US6] Add deadpool-postgres dependency to distributed-lock-postgres/Cargo.toml
- [X] T068 [US6] Add sha2 dependency for name hashing to distributed-lock-postgres/Cargo.toml
- [X] T069 [US6] Implement transaction-scoped lock support in crates/distributed-lock-postgres/src/lock.rs
- [X] T070 [US6] Implement keepalive mechanism for long-held locks in crates/distributed-lock-postgres/src/connection.rs
- [X] T071 [US6] Create Postgres integration test with testcontainers in tests/postgres_tests.rs
- [X] T072 [US6] Create lib.rs with public re-exports in crates/distributed-lock-postgres/src/lib.rs

---

## Phase 8: User Story 5 - Semaphore (P3)

**Goal**: Implement distributed semaphores allowing up to N concurrent holders.

**Independent Test**: Semaphore with max_count=3 allows exactly 3 processes to hold tickets simultaneously. 4th process blocks.

**Test Criteria**:
- Semaphore limits concurrent holders to max_count
- Releasing a ticket unblocks exactly one waiter
- Semaphore handles are independent

### Tasks

- [x] T073 [US5] Implement RedisDistributedSemaphore struct in crates/distributed-lock-redis/src/semaphore.rs
- [x] T074 [US5] Implement RedisSemaphoreHandle struct in crates/distributed-lock-redis/src/semaphore.rs
- [x] T075 [US5] Implement acquire for RedisDistributedSemaphore in crates/distributed-lock-redis/src/semaphore.rs
- [x] T076 [US5] Implement try_acquire for RedisDistributedSemaphore in crates/distributed-lock-redis/src/semaphore.rs
- [x] T077 [US5] Implement release for RedisSemaphoreHandle in crates/distributed-lock-redis/src/semaphore.rs
- [x] T078 [US5] Implement SemaphoreProvider for RedisLockProvider in crates/distributed-lock-redis/src/provider.rs
- [X] T079 [US5] Create semaphore integration test in tests/redis_tests.rs

---

## Phase 9: User Story 7 - Redis Backend (P3)

**Goal**: Implement Redis lock backend with RedLock algorithm for multi-server deployments and automatic lease extension.

**Independent Test**: Can acquire Redis locks, verify expiration/renewal behavior, and use RedLock with multiple servers.

**Test Criteria**:
- Redis locks expire correctly
- Lock extension works automatically
- RedLock requires majority consensus
- Lock handles detect lost locks

### Tasks

- [X] T080 [US7] Implement RedisLockProvider struct in crates/distributed-lock-redis/src/provider.rs
- [X] T081 [US7] Implement RedisLockProviderBuilder in crates/distributed-lock-redis/src/provider.rs
- [X] T082 [US7] Implement RedisDistributedLock struct in crates/distributed-lock-redis/src/lock.rs
- [X] T083 [US7] Implement RedisLockHandle struct in crates/distributed-lock-redis/src/handle.rs
- [X] T084 [US7] Implement RedisLockState struct in crates/distributed-lock-redis/src/lock.rs
- [X] T085 [US7] Implement single-server lock acquire using SET NX PX in crates/distributed-lock-redis/src/lock.rs
- [X] T086 [US7] Implement RedLock acquire algorithm in crates/distributed-lock-redis/src/redlock/acquire.rs
- [X] T087 [US7] Implement lock extension Lua script in crates/distributed-lock-redis/src/redlock/extend.rs
- [X] T088 [US7] Implement lock release Lua script in crates/distributed-lock-redis/src/redlock/release.rs
- [X] T089 [US7] Implement background lease extension task in crates/distributed-lock-redis/src/handle.rs
- [X] T090 [US7] Implement handle lost detection via lease expiry in crates/distributed-lock-redis/src/handle.rs
- [X] T091 [US7] Add fred dependency to distributed-lock-redis/Cargo.toml
- [X] T092 [US7] Add rand dependency for lock IDs to distributed-lock-redis/Cargo.toml
- [X] T093 [US7] Implement RedisDistributedReaderWriterLock in crates/distributed-lock-redis/src/rw_lock.rs
- [X] T094 [US7] Create Redis integration test with testcontainers in tests/redis_tests.rs
- [X] T095 [US7] Create lib.rs with public re-exports in crates/distributed-lock-redis/src/lib.rs

---

## Phase 10: User Story 8 - File Backend (P3)

**Goal**: Complete file system lock backend implementation (already started in Phase 3, but marked P3 for full feature set).

**Independent Test**: File locks work across processes on same machine/shared filesystem. OS-level locking prevents concurrent access.

**Test Criteria**:
- Lock files are created correctly
- OS-level locking prevents concurrent access
- Lock files are cleaned up on release
- Invalid names are handled safely

### Tasks

- [X] T096 [US8] Add comprehensive file lock error handling in crates/distributed-lock-file/src/lock.rs
- [X] T097 [US8] Add cross-platform file locking tests (Linux, macOS, Windows) in tests/file_tests.rs
- [X] T098 [US8] Implement busy-wait retry logic with exponential backoff in crates/distributed-lock-file/src/lock.rs

---

## Phase 11: Polish & Cross-Cutting

**Goal**: Meta-crate, documentation, benchmarks, CI/CD, and publishing preparation.

**Independent Test**: All crates compile, tests pass, documentation generates, benchmarks run.

### Tasks

- [X] T099 Create distributed-lock meta-crate lib.rs with re-exports in crates/distributed-lock/src/lib.rs
- [X] T100 Add all backend dependencies to meta-crate in crates/distributed-lock/Cargo.toml
- [X] T101 Add public API documentation to all traits and types
- [X] T102 Create README.md with usage examples at repository root
- [X] T103 Add tracing spans for acquire/release operations in all backends
- [X] T104 Create benchmark suite for lock acquisition latency in benches/
- [X] T105 Add clippy and rustfmt configuration to workspace
- [X] T106 Set up CI/CD pipeline with GitHub Actions
- [X] T107 Add testcontainers dependency for integration tests in tests/Cargo.toml
- [X] T108 Create examples directory with usage examples
- [X] T109 Prepare crates.io metadata (description, keywords, license)
- [X] T110 Run full test suite and fix any issues
- [X] T111 Verify MSRV 1.85 compatibility

---

## Implementation Strategy

### MVP First (Phases 1-4)

The MVP delivers working exclusive locks with async support using the file backend. This provides immediate value and validates the core architecture before adding database backends.

**MVP Deliverable**: `distributed-lock-core` + `distributed-lock-file` crates with full async mutex support.

### Incremental Delivery

After MVP, backends can be added independently:
1. **Postgres** (Phase 7) - Most production-ready, no external infrastructure needed if DB exists
2. **Redis** (Phase 9) - Popular choice, adds semaphore support
3. **Reader-Writer Locks** (Phase 6) - Extends Postgres backend
4. **Semaphores** (Phase 8) - Extends Redis backend

### Parallel Workstreams

Once core traits are stable (Phase 2), backend implementations can proceed in parallel:
- File backend (simplest, good for MVP)
- Postgres backend (most production-ready)
- Redis backend (most feature-rich)

---

## Task Summary

| Phase | User Story | Task Count | Priority |
|-------|------------|------------|----------|
| 1 | Setup | 8 | Required |
| 2 | Foundational | 14 | Required |
| 3 | US1 - Exclusive Lock | 13 | P1 |
| 4 | US2 - Async Lock | 6 | P1 |
| 5 | US3 - Provider Abstraction | 4 | P2 |
| 6 | US4 - Reader-Writer Lock | 9 | P2 |
| 7 | US6 - Postgres Backend | 18 | P2 |
| 8 | US5 - Semaphore | 7 | P3 |
| 9 | US7 - Redis Backend | 16 | P3 |
| 10 | US8 - File Backend | 3 | P3 |
| 11 | Polish | 13 | Required |

**Total**: 111 tasks

---

## Notes

- All tasks follow the checklist format: `- [ ] [TaskID] [P?] [Story?] Description with file path`
- Tasks marked `[P]` can be parallelized (different files, no dependencies)
- Tasks marked `[US1]`, `[US2]`, etc. belong to specific user stories
- File paths are absolute from repository root or relative to crate root
- Each phase is independently testable
- MVP scope (Phases 1-4) delivers working distributed locks
