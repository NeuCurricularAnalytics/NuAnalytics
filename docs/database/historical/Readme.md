# Historical schema patches

These files migrated a database that was created *before* a schema change. They are kept
because `CHANGELOG.md` cites them as the upgrade step for the releases that introduced
those changes.

**A fresh install needs neither of them.** Both say so in their own headers, and every
object in them is already in `schema.sql`:

| File | Migrated | Superseded by |
|---|---|---|
| `schema-patch-v2.sql` | Adds `completions.major_num` and `institution_completion_totals` | `schema.sql` |
| `rls-patch.sql` | Moves to the auth-required RLS model (read *and* write need `auth.role() = 'authenticated'`) | `schema.sql` |

They were moved out of `docs/database/` on 2026-09-18: that directory's documented apply
order names five files, and having seven sitting in it invited the question of whether the
other two were being skipped by mistake. See `docs/database/setup.md` for the order that
does apply.
