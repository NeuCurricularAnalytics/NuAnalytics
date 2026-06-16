---
name: degree-update
description: Update or revise an existing NuAnalytics degree YAML or unified JSON file. Use when the user wants to modify requirements, add or remove courses, fix errors, or refresh a degree file against a new catalog version.
---

# Degree Update

Revise an existing NuAnalytics degree file — supports both `*.unified.json` and `*.yaml` formats.

## Workflow

1. Identify the target file (`degrees/*.unified.json` or `degrees/*.yaml`). If ambiguous, list candidates.
2. Read the current file content.
3. Validate the current state via `validate_degree`. Note all existing errors before editing.
4. Apply the user's requested changes (requirement edits, course additions/removals, metadata corrections).
5. Re-validate via `validate_degree`. All errors must be resolved before declaring done.
6. Run `audit_degree` and surface any new warnings introduced by the edits.

## Format notes

- **`*.yaml`** — human-authored YAML; prefer for hand-edited work.
- **`*.unified.json`** — machine-generated combined format; update by editing the source YAML and regenerating, or edit the JSON directly if that is the canonical source.
- Never silently convert between formats — preserve whichever format the file already uses unless the user explicitly requests a conversion.

## Critical rules

- Verify every changed course number against the source catalog.
- Show credit arithmetic for any section whose totals change.
- Do not remove requirements unless the user explicitly confirms the deletion.
