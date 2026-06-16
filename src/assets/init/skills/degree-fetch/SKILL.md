---
name: degree-fetch
description: Fetch and build a NuAnalytics degree file from the database with dual-build verification. Use when the user wants to pull a degree from the database and save it as a *.unified.json or *.yaml file.
---

# Degree Fetch

Fetch a degree from the database with dual-build verification to catch transcription errors.

## Workflow

1. Ask the user for the desired output format: `*.unified.json` or `*.yaml`. Default to `*.yaml` if unspecified.
2. Identify the degree via `search_degrees`. Confirm the match with the user before proceeding.
3. **Dual build** — execute two independent `scaffold_degree_yaml` calls in parallel, writing each result to a scratch file (`degrees/_fetch_a.<ext>` and `degrees/_fetch_b.<ext>`).
4. **Compare** — diff the two outputs:
   - If identical: proceed to step 5.
   - If discrepancies exist: apply majority-rules reconciliation — values that agree between both builds are accepted automatically; any field where the two builds differ must be surfaced to the user for confirmation before the final file is written.
5. Write the reconciled result to `degrees/<institution>-<program>.<ext>`.
6. Delete both scratch files.
7. Validate the final file via `validate_degree`. Fix any errors before declaring done.

## Format notes

- **`*.unified.json`** — use when downstream tools expect the combined JSON format.
- **`*.yaml`** — use for human-readable, hand-editable output.

## Critical rules

- Never skip dual-build verification, even for small degree files.
- Do not auto-resolve discrepancies silently — always surface them to the user.
- Always delete scratch files after reconciliation, whether it succeeded or was aborted.
