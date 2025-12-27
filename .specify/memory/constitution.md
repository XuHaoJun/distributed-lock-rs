<!--
  Sync Impact Report
  ==================
  Version change: 0.0.0 → 1.0.0 (initial constitution)
  
  Added principles:
    - I. MVP-First Delivery
    - II. Code Quality Standards
    - III. Testing Discipline
    - IV. API Consistency
    - V. Performance by Design
  
  Added sections:
    - Core Principles (5 principles)
    - Simplicity Constraints
    - Quality Gates
    - Governance
  
  Removed sections: None (initial creation)
  
  Templates verification:
    ✅ plan-template.md - Constitution Check section exists, compatible with principles
    ✅ spec-template.md - MVP prioritization in user stories aligns with Principle I
    ✅ tasks-template.md - MVP-first strategy and independent testing aligns with Principles I & III
  
  Follow-up TODOs: None
-->

# distributed-lock-rs Constitution

## Core Principles

### I. MVP-First Delivery

Every feature MUST be delivered incrementally starting with the minimum viable implementation.

- Implement P1 (highest priority) user stories first; validate before adding P2/P3
- Each user story MUST be independently functional and testable
- Defer "nice to have" features until core functionality is proven
- Stop and ship when the minimum requirement is met—resist scope creep
- If a feature can be split, split it; if a decision can be deferred, defer it

**Rationale**: Shipping working software early exposes real requirements and reduces wasted effort on unused features.

### II. Code Quality Standards

All code MUST meet quality standards that enable long-term maintenance.

- Code MUST compile without warnings (treat warnings as errors in CI)
- All public APIs MUST have documentation comments
- Functions MUST be small and single-purpose; if it needs a comment explaining "what," refactor
- Naming MUST be clear and consistent across the codebase
- Dependencies MUST be justified; prefer standard library over external crates when equivalent

**Rationale**: Quality code reduces debugging time, enables confident refactoring, and makes onboarding faster.

### III. Testing Discipline

Tests MUST validate behavior at the boundaries where failures matter most.

- Integration tests MUST cover all public API entry points
- Contract tests MUST verify compatibility between components
- Unit tests for complex logic only—skip trivial getters/setters
- Tests MUST be fast (<100ms per test) or explicitly marked as slow
- Flaky tests MUST be fixed immediately or deleted; never ignored

**Rationale**: Tests exist to catch regressions, not to hit coverage metrics. Focus testing effort where bugs actually occur.

### IV. API Consistency

User-facing APIs MUST provide a consistent, predictable experience.

- All lock types MUST share the same trait signatures (acquire, try_acquire, acquire_async)
- Error types MUST be consistent across backends; users should not handle backend-specific errors
- Naming conventions MUST follow Rust idioms: snake_case for functions, CamelCase for types
- Breaking changes MUST follow semantic versioning; deprecate before removing

**Rationale**: Consistent APIs reduce cognitive load and enable users to apply knowledge across the library.

### V. Performance by Design

Performance MUST be considered during design, not bolted on afterward.

- Async operations MUST NOT block the runtime; use spawn_blocking for CPU-bound work
- Lock acquisition MUST complete within timeout + minimal overhead (<10ms)
- Memory allocations in hot paths MUST be minimized; prefer stack allocation
- Benchmarks MUST exist for critical paths; regressions block merge
- Connection pooling MUST be used for database backends

**Rationale**: A distributed lock library that adds significant latency defeats its purpose.

## Simplicity Constraints

Complexity MUST be justified. The following constraints prevent overdesign:

- **No speculative features**: Do not implement functionality "because we might need it"
- **No premature abstraction**: Wait for three concrete use cases before creating an abstraction
- **No gold plating**: If it works and meets requirements, stop improving it
- **Maximum 3 layers**: Code paths should traverse at most 3 abstraction layers (e.g., API → Service → Backend)
- **Reject complexity debt**: If adding a feature requires "temporary" complexity, reconsider the feature

**Violation escalation**: Any complexity beyond these constraints MUST be documented in the plan with:
1. Why simpler alternatives are insufficient
2. The specific problem being solved
3. Agreement to revisit if assumptions prove wrong

## Quality Gates

All pull requests MUST pass these gates before merge:

| Gate | Requirement |
|------|-------------|
| Build | Compiles on stable Rust without warnings |
| Tests | All tests pass; no new flaky tests |
| Docs | Public APIs documented; README updated if behavior changes |
| Lint | `cargo clippy` passes with no warnings |
| Format | `cargo fmt --check` passes |

**Pre-commit checklist** (manual verification):
- [ ] I have run the tests locally
- [ ] I have not added unnecessary dependencies
- [ ] I have not introduced breaking changes without version bump

## Governance

This constitution supersedes conflicting guidance in other documents. Amendments require:

1. Written proposal describing the change and rationale
2. Update to this document with version increment
3. Verification that dependent templates remain consistent

**Version policy**:
- MAJOR: Principle removal or fundamental redefinition
- MINOR: New principle or significant expansion of existing guidance
- PATCH: Clarification, typo fix, or non-semantic refinement

**Compliance**: All PRs and code reviews MUST verify adherence to these principles. Non-compliance MUST be justified in the PR description.

**Version**: 1.0.0 | **Ratified**: 2025-12-27 | **Last Amended**: 2025-12-27
