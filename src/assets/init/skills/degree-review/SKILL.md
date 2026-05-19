---
name: degree-review
description: Review or validate an existing NuAnalytics degree YAML. Use when the user shares a degree YAML file and asks for feedback, validation, or an audit — or when a YAML in `degrees/` has changed.
---

# Degree Review

Validate and critique an existing degree YAML.

## Workflow

1. Read the target YAML file.
2. Call the `validate_degree` MCP tool. Report every error.
3. Call the `audit_degree` MCP tool. Group warnings by category (prerequisite chains, cross-listings, credit totals, structural).
4. Cross-check against the source catalog if the user provided one.
5. For each finding, propose the exact fix (file:line or block, with the corrected YAML).

## What to look for beyond the tools

- Requirement types that don't match the catalog's intent ("all" vs "one_of").
- Missing electives, missing prereqs, dead-end requirements.
- Credit totals that don't sum to the stated program total.
- Course numbers that look like typos (e.g., CS 100 vs CS 1000).
