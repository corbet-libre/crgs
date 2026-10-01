# Coverage contract

CI targets 100% of reachable production lines and branches. Stable Rust runs
the existing native and wasm checks. Nightly Rust is used only for LLVM branch
instrumentation, which currently requires it. Both jobs use the same resolved
Cargo.lock snapshot; all build and test commands after resolution use --locked.

The coverage job executes real tests with cargo-llvm-cov and retains the raw JSON
even when the gate fails. The checker compares integer covered/total counts for
both metrics; rounded percentages cannot pass. An empty report cannot pass.
The report excludes integration-test harness files under tests/, not production
code. No production source exclusions are currently approved.

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

These three emitted lines and two branch arms require independent review. They
are not broader exclusions for transaction, schema, parsing or storage failures.
The manifest binds each exact coordinate and text to its entire source file.
Any source change invalidates the proposal. Raw reports retain every counter.

- `register.rs:192` false and its emitted closing line198: `extend_lease`
  returns immediately at175–176 for a member without a handle. The owned
  `StoredMember` has no shared mutable interior. No statement between that guard
  and192 modifies its handle, including the earlier-return release path.
- `register.rs:236` false and its emitted closing line239: `change_handle`
  returns at215–216 if the owned handle is absent. `previous` clones exactly
  that handle at218; no statement modifies `previous`. Storage awaits cannot
  mutate the owned clone. Real claim conflicts and transaction failures remain
  covered separately.
- `sql.rs:119`: the fixed `RESERVATION` query at15 filters `kind = 'reserved'`
  before projecting that same column. `decode_claim` maps exactly this string
  to `Claim::Reserved` or returns an actual malformed-field error. Under the
  database query contract the match cannot receive another successful variant.
  The tests exercise real malformed active/unknown claims through the unfiltered
  claim query and actual reserved-record decoding failures; they do not forge
  driver rows to defeat the fixed SQL predicate.

Imported unchecked columns and inconsistent owner/handle claims use real
libSQL/Memory transactions and preserve rollback on refusal. No domain guard
was removed or weakened to increase coverage. This facade has native storage;
its CI does not claim device or whole-stack browser qualification.
