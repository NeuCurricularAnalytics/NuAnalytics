---
name: check-rs
description: Rust Code Quality Pass — fmt, clippy, DRY/complexity/test review agents, then fixes all findings
user-invocable: true
---

# /check-rs — Rust Code Quality Pass

Perform a comprehensive quality pass on recently changed Rust code. Fix every issue found — do not just report them.

## Phase 1: Identify what changed

Run `git diff HEAD --stat` and `git status --short` to identify the changed and untracked files. Exclude `Cargo.lock`. If there are no git changes, ask the user which files to review.

## Phase 2: Run the tooling (fix before reviewing)

Run these commands and fix every failure before proceeding. Do not skip to the agents if either tool reports problems.

**Formatting:**
```
cargo fmt
```
Apply formatting automatically — do not just check.

**Linting:**
```
cargo clippy -- -D warnings
```
Warnings are treated as errors. Fix every warning in the changed files (and any file you touch while fixing). Do not use `#[allow(...)]` to suppress a warning unless the suppression has a doc comment explaining why it is necessary.

After fixing all clippy issues, run `cargo build` to confirm a clean compile with zero warnings.

## Phase 3: Launch three review agents in parallel

Use the Agent tool to launch all three agents **in a single message** (parallel). Pass each agent the list of changed files and their full content.

### Agent 1: DRY + Organisation Review
For each changed file:
1. Search for existing utilities, helpers, and patterns elsewhere in the codebase that could replace newly written code. Check adjacent modules, utility files, and the nearest `mod.rs`.
2. Flag any function that duplicates existing functionality and name the existing alternative.
3. Flag inline logic that re-implements something already in the standard library or a crate already in `Cargo.toml`.
4. Flag modules or types that belong in a different file/module based on the existing project structure.
5. Flag any string literal that should be a named constant based on how it is used.

Report: file:line, what duplicates what, the exact fix.

### Agent 2: Complexity + Documentation Review
For each changed file:
1. **Long functions** — flag any function over ~40 lines that could be split. Suggest what the extracted helper should be called and what it should contain.
2. **Deep nesting** — flag `match`/`if`/`for` blocks nested more than 3 levels deep. Suggest early-return or helper extraction.
3. **Missing docs** — flag every `pub` item (fn, struct, enum, const, mod) that lacks a doc comment. Suggest the doc text based on the item's name and usage.
4. **Useless comments** — flag comments that describe *what* the code does (well-named code already does that), narrate the change, or reference the task. Keep only *why* comments (hidden constraints, non-obvious invariants, workarounds).
5. **Overly broad error messages** — error strings that don't identify the context (file, field, operation).

Report: file:line, the issue, the exact fix.

### Agent 3: Test Coverage Review
For each changed file:
1. List every `pub fn`, `pub async fn`, and non-trivial private fn in the file.
2. For each function, determine whether a test exists that exercises its main path and at least one edge case (empty input, `None`, boundary value, error path).
3. Flag functions with no test and functions with only a happy-path test.
4. Check that the test module uses `#[cfg(test)]` and that test names describe the scenario (`test_<function>_<scenario>`).
5. Check that pure/deterministic functions (parsers, validators, formatters) have property-style tests (multiple inputs in one test).

Report: file:function, what test is missing, a concrete test to add (include the code).

## Phase 4: Fix all findings

Wait for all three agents to complete. Work through every finding:
- Apply the fix directly using Edit/Write tools.
- If a finding is a clear false positive, note it and skip.
- Do not add `#[allow(...)]` without a doc comment explaining the suppression.
- Do not add tests that test nothing (trivial getters, `Default::default()` calls).

## Phase 5: Final verification

Run in sequence:
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

All three must pass cleanly. If any fail, fix and re-run before declaring done.

## Summary

Report what was fixed in each category:
- **Tooling** — fmt/clippy changes made
- **DRY / Organisation** — duplications removed, constants added, code moved
- **Complexity / Docs** — functions split, docs added, comments removed
- **Tests** — new tests added (list them)

If a category had no issues, say so explicitly.
