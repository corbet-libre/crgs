# Coverage contract

CI targets 100% of reachable production lines and branches. Stable Rust runs
the existing native and wasm checks. Nightly Rust is used only for LLVM branch
instrumentation, which currently requires it. Both jobs use the same resolved
Cargo.lock snapshot; all build and test commands after resolution use --locked.

The coverage job executes real tests with cargo-llvm-cov and retains the raw JSON
even when the gate fails. The checker compares integer covered/total counts for
both metrics; rounded percentages cannot pass. An empty report cannot pass.
The report excludes integration-test harness files under tests/, not production
code. Only the exact reviewed proposals below are subtracted; there is no
blanket or file-level exclusion.

A failing gate is missing evidence, not permission to lower the threshold or
change domain behavior. Add meaningful failure and round-trip tests. Document
any genuinely unreachable defensive branch precisely before excluding it. Native
coverage does not establish browser execution; keep the actual wasm vectors.

First-party dependencies follow main. Their resolved full revisions remain in
Cargo.lock, with exactly one source per first-party crate. Dependabot maintains
committed snapshots; CI refreshes once per run and retains the tested snapshot.
Auto-merge requires protected main and successful substantive checks on the
exact current Dependabot head. It never executes PR code with write permissions.

## Emitted source metric

The acceptance metric is now 100% of upstream LLVM LCOV's emitted production
source line counters (DA) and branch counters (BRDA), from the same execution as
the retained raw JSON. This is source coverage, not every generic instantiation.
LLVM JSON and LCOV summary totals can count generic copies differently from
the merged source records. The original JSON checker remains diagnostic code;
its stricter instantiation totals are not described as passed.

The source gate requires a nonempty report, an exact file inventory matching the
companion JSON, matching raw summary metadata, complete emitted branch counts
and no duplicate, unknown or zero counters. Only the exact proposals below may be subtracted after all inventories reconcile.
Actual browser/device execution remains separate from native coverage.

## Proposed invariant exceptions

These four emitted lines and two branch arms require independent review. They
are not broader exclusions for transaction, schema, parsing or storage failures.
The manifest binds each exact coordinate and text to its entire source file.
Any source change invalidates the proposal. Raw reports retain every counter.

- `register.rs:190` false arm and its emitted closing line `193`:
  `change_handle` returns at 169–171 for a member without a handle. `previous`
  clones exactly that guard-passed handle at 172 and no statement writes the
  owned clone, so the `Some` pattern cannot fail. Real claim conflicts and
  transaction failures remain covered separately.
- `register/renewal.rs:46` false arm and its emitted closing line `52`:
  `stage_renewal` returns at 31–33 for a member without a handle. The owned
  `StoredMember` has no shared mutable interior and no await between that guard
  and this condition writes it, so the condition cannot be false.
- `register/first.rs:17`: LLVM's only region on this line is the error edge of
  `into_owned_transaction`. `ensure_empty_in_transaction` just wrapped its own
  write transaction, so the wrapper is neither an advisory read capability nor
  one that recorded a decode failure. Both guards of that conversion are
  asserted directly
  (`sql::tests::a_failed_decode_refuses_to_yield_an_owned_writer`,
  `sql::tests::an_advisory_read_cannot_become_an_owned_write_transaction`).
- `sql.rs:141`: the fixed `RESERVATION` query at 15 filters `kind = 'reserved'`
  before projecting that same column. `decode_claim` maps exactly this string
  to `Claim::Reserved` or returns an actual malformed-field error. Under the
  database query contract the match cannot receive another successful variant.
  The tests exercise real malformed active/unknown claims through the unfiltered
  claim query and actual reserved-record decoding failures; they do not forge
  driver rows to defeat the fixed SQL predicate.

A fully acknowledged role change (`set_role_checked` with both releases granted)
is covered as real behaviour, not excluded.

Imported unchecked columns and inconsistent owner/handle claims use real
libSQL/Memory transactions and preserve rollback on refusal. No domain guard
was removed or weakened to increase coverage. This facade has native storage;
its CI does not claim device or whole-stack browser qualification.
