# crgs agent notes

Work in a lane (`~/agents/scratch/lane-new <lane>`); never edit `~/agents/scratch/ws`.
Run commands from the lane root (`<lane>/ws`); `<lane>` below is your lane name.

Build and test (quote the verdict line, never infer from a pipe):
- `~/agents/scratch/cbuild <lane> test -p crgs` — green, exit 0
  (lib + 7 integration suites `ok`; `tests/database.rs::optional_turso_round_trip`
  skips without `TURSO_URL`+`TURSO_TOKEN` against a disposable test database).
- `cargo fmt -p crgs -- --check` — clean at HEAD.
- `clippy -p crgs --all-targets -- -D warnings` is red at HEAD from a
  pre-existing drift (`src/sql.rs:72` large size difference between `Connection`
  variants; local rustc 1.98.0). Fix code, never lint.
- Full gate (locked deps, coverage) runs in CI only: `.ci/verify.sh`.
  Lane `--locked` needs CI's resolve refresh.

Land: `cargo fmt` in touched repos, rebase on main (never force-push),
`cbuild <lane> xtask land --dry`, then `xtask land`.

Invariants (rule -> enforcing test):
- Every operation is scoped by community, even in one shared database
  -> `tests/database.rs::every_operation_is_scoped_even_when_a_database_is_shared`
- Foreign scopes cannot join, supply members, or renew
  -> `tests/admission_transaction.rs::different_scope_or_separately_opened_same_url_cannot_join_this_register`,
  `tests/member_transaction.rs::foreign_scope_or_independent_same_url_database_handle_cannot_supply_member_data`
- Admission commits only with the caller; exact retry survives reopen
  -> `tests/admission_transaction.rs::joined_admission_commits_only_when_caller_commits_and_exact_retry_survives_reopen`
- Supplied skeletons must equal UTS #39 of the normalized display
  -> `tests/register.rs::supplied_skeletons_must_match_normalized_displays`
- Pending reservation is not membership; retry never slides its deadline
  -> `tests/first_transaction.rs::pending_reservation_is_not_a_member_but_a_retained_legacy_member_is_not_fresh_creation`
- Callers supply the trusted UTC clock; crgs never reads a system clock
  (`README.md:60-62`); `forbid(unsafe_code)` (`src/lib.rs`).
- Storage only through `crlt`; one database per community.

Pointers: `src/register.rs`, `src/sql.rs`, `tests/`, `docs/CONTRACT.md`.
Commits: explicit paths, imperative, author per `~/agents/AGENT.md`; no AI trailers.
`publish=false`; LGPL-3.0-only WITH linking exception; no registry publication.
