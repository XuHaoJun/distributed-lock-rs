# Feature Specification: Port DistributedLock to Rust

**Feature Branch**: `001-port-distributed-lock`  
**Created**: 2025-12-27  
**Status**: Draft  
**Input**: User description: "Port DistributedLock C# library to Rust"

## Clarifications

### Session 2025-12-27

- Q: Should the library provide built-in observability (logging/tracing)? → A: Structured tracing hooks via `tracing` crate (span on acquire/release)
- Q: How should TLS/SSL be handled for Postgres/Redis connections? → A: Support TLS via connection config; document as recommended for production
- Q: What is the maximum acceptable lock acquisition overhead? → A: <10ms beyond backend latency
- Q: What queuing discipline for lock waiters? → A: FIFO (first-in-first-out) - waiters served in request order
- Q: What is the minimum supported Rust version (MSRV)? → A: MSRV 1.75 - enables native async traits

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Acquire Exclusive Lock on Shared Resource (Priority: P1)

A developer building a distributed system needs to protect a critical section of code from concurrent execution across multiple processes or machines. They create a distributed lock with a unique name, acquire it before entering the critical section, and release it when done.

**Why this priority**: Exclusive locking is the core functionality of the library. Without it, no other features have value. This is the minimum viable product.

**Independent Test**: Can be fully tested by creating a lock, acquiring it, verifying exclusivity, and releasing it. Delivers immediate value for protecting critical sections.

**Acceptance Scenarios**:

1. **Given** a distributed lock with name "resource-x", **When** process A acquires the lock, **Then** process A holds exclusive access and process B's acquire attempt blocks until A releases.
2. **Given** a held lock, **When** the lock handle is dropped/disposed, **Then** the lock is released and other waiters can acquire it.
3. **Given** an available lock, **When** a process calls try_acquire with zero timeout, **Then** it returns a handle immediately if available or None if held elsewhere.
4. **Given** a held lock, **When** another process calls acquire with a timeout, **Then** it waits up to the timeout duration before returning an error.

---

### User Story 2 - Async Lock Acquisition (Priority: P1)

A developer building an async Rust application needs to acquire distributed locks without blocking the async runtime. They use async acquire methods that yield while waiting for the lock.

**Why this priority**: Rust's async ecosystem is fundamental to modern systems programming. Async support is essential for the library to be useful in real-world applications.

**Independent Test**: Can be tested with async tests that verify lock acquisition completes via await without blocking threads.

**Acceptance Scenarios**:

1. **Given** an async runtime, **When** a task calls acquire_async on an available lock, **Then** it obtains the lock without blocking the thread.
2. **Given** a held lock, **When** multiple async tasks wait on acquire_async, **Then** they are served in FIFO order when the lock becomes available.
3. **Given** an async acquire in progress, **When** a cancellation signal is received, **Then** the acquire attempt is cancelled and returns an appropriate result.

---

### User Story 3 - Backend-Agnostic Lock Provider (Priority: P2)

A team wants to write application code that is decoupled from the specific distributed lock backend (Redis, Postgres, file system, etc.). They inject a lock provider that creates locks by name, allowing the backend to be configured at startup or swapped for testing.

**Why this priority**: Provider abstraction enables dependency injection patterns and makes testing easier by allowing mock implementations.

**Independent Test**: Can be tested by implementing the provider trait for a mock backend and verifying lock creation works identically.

**Acceptance Scenarios**:

1. **Given** a lock provider configured with a backend, **When** create_lock("name") is called, **Then** it returns a lock instance using that backend.
2. **Given** application code using the provider trait, **When** a different backend is configured, **Then** the application code works without modification.

---

### User Story 4 - Reader-Writer Lock for Concurrent Reads (Priority: P2)

A developer has a resource that can be safely read concurrently but requires exclusive access for writes. They use a distributed reader-writer lock that allows multiple readers OR a single writer.

**Why this priority**: Reader-writer locks are a common synchronization pattern that significantly improves throughput for read-heavy workloads.

**Independent Test**: Can be tested by having multiple processes acquire read locks concurrently, then verifying write lock acquisition blocks all readers.

**Acceptance Scenarios**:

1. **Given** a reader-writer lock with no holders, **When** multiple processes request read locks simultaneously, **Then** all are granted concurrently.
2. **Given** active read locks, **When** a process requests a write lock, **Then** it blocks until all readers release.
3. **Given** an active write lock, **When** processes request read or write locks, **Then** they all block until the writer releases.
4. **Given** waiting readers and writers, **When** a write lock is released, **Then** writers are given priority to prevent writer starvation.

---

### User Story 5 - Distributed Semaphore for Throttling (Priority: P3)

A developer needs to limit concurrent access to a resource (like a database connection pool) across distributed processes. They use a semaphore with a max count to ensure no more than N processes access the resource simultaneously.

**Why this priority**: Semaphores extend the library's utility beyond mutual exclusion but are less commonly needed than basic locks.

**Independent Test**: Can be tested by creating a semaphore with max_count=3 and verifying exactly 3 processes can hold tickets simultaneously.

**Acceptance Scenarios**:

1. **Given** a semaphore with max_count of 5, **When** 5 processes acquire tickets, **Then** all succeed and a 6th blocks.
2. **Given** a fully-utilized semaphore, **When** one ticket is released, **Then** exactly one waiting process is unblocked.

---

### User Story 6 - Postgres Advisory Lock Backend (Priority: P2)

A developer using PostgreSQL wants to leverage the database's advisory locks for distributed synchronization. They configure the library with a Postgres connection and use advisory locks under the hood.

**Why this priority**: Postgres is widely used, and advisory locks are a robust, proven mechanism. This provides a production-ready backend without additional infrastructure.

**Independent Test**: Can be tested with a Postgres instance by acquiring locks and verifying they are exclusive across connections.

**Acceptance Scenarios**:

1. **Given** a Postgres connection string, **When** a lock is created and acquired, **Then** a Postgres advisory lock is held on the database.
2. **Given** an advisory lock key (64-bit integer or name), **When** the lock is acquired, **Then** the correct pg_advisory_lock is called.
3. **Given** a held lock, **When** the database connection is lost, **Then** the lock handle reflects the lost state via a notification mechanism.

---

### User Story 7 - Redis Lock Backend with RedLock (Priority: P3)

A developer using Redis wants distributed locks with high availability. They configure multiple Redis instances and the library uses the RedLock algorithm for consensus-based locking.

**Why this priority**: Redis is a popular choice for distributed coordination. RedLock provides fault tolerance but is more complex to implement.

**Independent Test**: Can be tested with a Redis instance by acquiring locks and verifying expiration/renewal behavior.

**Acceptance Scenarios**:

1. **Given** a Redis database connection, **When** a lock is acquired, **Then** a key is set with an expiration time.
2. **Given** a held Redis lock, **When** the lock is held beyond initial expiry, **Then** the library automatically extends the lock.
3. **Given** multiple Redis databases, **When** a lock is acquired, **Then** it must succeed on a majority of databases (RedLock algorithm).

---

### User Story 8 - File System Lock Backend (Priority: P3)

A developer needs distributed locking on a single machine or shared filesystem without external dependencies. They use file-based locks that work across processes.

**Why this priority**: File locks have zero infrastructure requirements and are useful for simpler deployments or local development.

**Independent Test**: Can be tested by creating lock files and verifying OS-level file locking prevents concurrent access.

**Acceptance Scenarios**:

1. **Given** a directory path, **When** a lock with name "foo" is acquired, **Then** a lock file is created and exclusively locked.
2. **Given** a held file lock, **When** another process attempts to acquire, **Then** it blocks or fails based on timeout.

---

### Edge Cases

- What happens when a lock holder crashes without releasing? Backend-specific mechanisms (connection close, key expiry, file handle cleanup) ensure eventual release.
- What happens when network connectivity is lost while holding a lock? The handle provides a "lost token" notification mechanism so the holder knows the lock may be compromised.
- What happens when acquire times out? A timeout error is returned; no lock is held.
- What happens when the same process tries to acquire a lock it already holds? The lock is non-reentrant; it blocks or fails depending on backend behavior.
- What happens when invalid lock names are provided? Names are validated or safely escaped/hashed to ensure backend compatibility.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Library MUST provide exclusive (mutex) distributed locks that prevent concurrent access to protected resources across processes and machines.
- **FR-002**: Library MUST provide both blocking acquire (waits until lock available or timeout) and try-acquire (returns immediately if unavailable) semantics.
- **FR-003**: Library MUST support configurable timeouts for acquire operations.
- **FR-004**: Library MUST release locks automatically when handles are dropped (RAII pattern).
- **FR-005**: Library MUST provide async variants of all acquire operations compatible with Rust's async/await.
- **FR-006**: Library MUST support cancellation of acquire operations via cancellation tokens or async cancellation.
- **FR-007**: Library MUST provide a trait-based provider abstraction for creating locks by name, enabling dependency injection and backend swapping.
- **FR-008**: Library MUST provide reader-writer locks allowing multiple concurrent readers OR one exclusive writer.
- **FR-009**: Library MUST provide semaphores that allow up to N concurrent holders.
- **FR-010**: Library MUST provide a Postgres advisory lock backend implementation.
- **FR-011**: Library MUST provide a Redis lock backend implementation.
- **FR-012**: Library MUST provide a file system lock backend implementation.
- **FR-013**: Library MUST handle lock name validation/escaping to ensure compatibility with backend restrictions.
- **FR-014**: Lock handles MUST provide a mechanism to detect if the underlying lock was lost (e.g., due to connection failure).
- **FR-015**: Library MUST ensure abandoned locks (process crash, GC without dispose) do not remain held indefinitely.
- **FR-016**: Library MUST be non-reentrant (a thread/task cannot acquire the same lock it already holds without deadlock).
- **FR-017**: Library MUST emit structured tracing spans via the `tracing` crate for acquire and release operations, enabling zero-cost observability when tracing is disabled.
- **FR-018**: Lock acquisition SHOULD serve waiting processes in FIFO order where the backend supports it.

### Key Entities

- **DistributedLock**: Represents a named mutex lock. Created via a provider or directly with backend-specific configuration. Has a unique name.
- **DistributedReaderWriterLock**: A lock supporting read (shared) and write (exclusive) acquisition modes. Has a unique name.
- **DistributedSemaphore**: A counting lock allowing up to max_count concurrent holders. Has a unique name and max_count.
- **LockHandle**: Returned when a lock is acquired. Holding this handle means holding the lock. Dropping releases the lock. Provides handle_lost notification.
- **LockProvider**: A factory trait that creates lock instances by name. Enables backend abstraction and dependency injection.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Developers can protect a critical section with exclusive distributed locking using simple acquire/drop patterns.
- **SC-002**: Library supports at least three backend technologies (Postgres, Redis, file system) with consistent API.
- **SC-003**: All lock types (mutex, reader-writer, semaphore) work correctly in async Rust applications without blocking the runtime.
- **SC-004**: Lock acquisition with timeout completes within the specified duration plus <10ms overhead.
- **SC-005**: Abandoned locks are automatically released within backend-specific timeframes (connection timeout, key expiry, etc.).
- **SC-006**: Applications can swap backend implementations without changing business logic code.
- **SC-007**: Lock handles provide reliable detection of lost locks within seconds of connection/lease failures.
- **SC-008**: Reader-writer locks allow concurrent readers without blocking each other.
- **SC-009**: Semaphores correctly limit concurrent access to the configured max count.
- **SC-010**: Library compiles and passes tests on Linux, macOS, and Windows.

## Assumptions

- Users have access to the backend infrastructure they choose (Postgres server, Redis server, or writable filesystem).
- Rust's async ecosystem (tokio or async-std) is used for async operations; tokio is the primary target runtime. Minimum supported Rust version (MSRV) is 1.75, enabling native async traits without the async-trait crate workaround.
- The library will follow Rust idioms (traits, RAII, Result types) rather than directly translating C# patterns.
- Connection pooling for database backends will leverage existing Rust crates (e.g., `deadpool`, `bb8`) rather than reimplementing.
- The library will be published as a Cargo crate (or workspace of crates) following the modular structure of the C# library.
- TLS/SSL connections are supported via connection configuration and documented as recommended for production deployments.