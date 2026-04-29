# AGENTS Guidelines

## Core Principle

- Mirror upstream GraphQL Code Generator behavior as closely as Rust allows.

## Always-On Rules

- Prioritize upstream parity first: structure, module boundaries, function responsibilities, and generated output shape.
- Keep naming aligned with upstream semantics; apply Rust conventions only where required (for example: snake_case, module file naming).
- Validate parity using `dev-test` generated fixtures before considering a task complete.
- Do not run destructive git commands.
- Do not create commits unless explicitly requested.

## Practical Expectations

- Prefer small, incremental edits that preserve upstream mental model.
- When behavior differs, reconcile output with upstream before refactoring for style.
- Keep transitional stubs clearly marked and named after upstream counterparts.
