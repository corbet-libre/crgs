# crgs contract

`crgs` is the community register below `cmbr` in `cvld`. It stores the
community pseudonym as opaque bytes, a display handle and caller-supplied
UTS #39 skeleton, role (`root`, `admin`, `member`), and a coarse lease end.
It implements reservation, admission, lookup, role changes, lease extension,
handle changes and expiry release. It contains no cryptography, login dates,
request logs, gate data, normalization or authorization engine.

## Caller boundary

The service chooses a separate database and fixed storage scope per community.
Every SQL table and unique key includes `community_id`; `crlt` injects the
scope. A shared-database test verifies defence in depth. Pseudonyms are never
derived or compared across communities. The caller authenticates the member,
authorizes roles and changes, validates handles with its handle checker, and
supplies a trusted UTC clock and policy. A supplied skeleton is treated as an
exact, case-sensitive string; crgs cannot verify its relationship to a display
handle. Never expose this storage capability directly to untrusted clients.

## Time and retention

- `YearMonth` accepts years 1–9999 and months 1–12. A lease is valid through
  the end of its stated month. No day or time is stored in a member record.
- `ReleasePeriod` is a nonnegative number of months, default 24. Release starts
  on the first day following `lease_end + period`. Thus a September 2026 lease
  with the default period frees its handle on 1 October 2028.
- A changed skeleton is blocked until the first day following
  `change_month + period`. Rounding conservatively to the next month avoids
  saving the change instant or releasing early. A display-only change with
  the same skeleton does not create a block.
- Deadlines are materialized when admitting, extending or changing, so a
  later policy change cannot shorten a committed deadline. Lease extension
  never shortens either the lease or its release deadline.
- Pending reservation expiry is an explicit whole UTC second supplied by the
  caller. It is transient enrolment state, removed on admission, cancellation
  or expiry. Subsecond deadlines are rejected. No creation/last-use time is
  recorded. The caller limits reservation lifetime and throttles requests.
- Release removes the current handle from the member, retaining only ID,
  role and lease. A block retains only the skeleton and its coarse deadline,
  with no former owner or display handle. There is no handle history.

## Atomic operations and retries

`Register<S>` contains the shared rules; `Storage` and `Transaction` are a
small transactional persistence boundary. Implementations must isolate all
operations until commit and roll back errors, cancellation and dropped
transactions. `MemoryStorage` is a real in-memory implementation;
`LibsqlStorage` uses `crlt` immediate transactions and the official libSQL
client. No separate implementation of register rules is hidden in a test mock.

One claim table covers pending, active and blocked skeletons with a unique
`(community_id, skeleton)` primary key. One member can have at most one pending
reservation. Admission requires the matching unexpired reservation, including
its display handle. Repeating an identical reservation or admission is safe;
a reservation retry never slides its deadline. Changing to the current handle
is safe and does not slide any block deadline. Different admission data for an
existing member is a conflict. Explicit role changes and monotonic lease
extensions are independent operations.

Reads do not clean up or record access. Handle lookup and availability apply
the supplied current time even before maintenance. Writes reclaim an expired
target claim atomically. `release_expired` processes bounded batches per claim
kind, and may be repeated until all counts are zero. Release never resurrects
an old handle: extending a released member only updates its lease; a separate
handle change can claim a new handle once the lease is current. Reserving a
handle is for pending enrolment, not existing members.

## Schema and integration

`SCHEMA` is trusted DDL for the service's combined `crlt` migration history.
`LibsqlStorage` takes an already migrated `crlt::Community`; it neither opens
connections nor owns migration numbering. Tables have `crgs_` prefixes.
Member and skeleton lookups use composite primary keys; reservation-owner
lookups and expiry sweeps have dedicated community-first indexes. Query-plan
checks cover all SQL statements used by the adapter. All values are bound
parameters. Storage failures expose no SQL, handles, IDs, URLs or tokens.
Remote failures can have uncertain commit outcomes; reconcile through reads
before retrying. crgs does not automatically retry database failures.

The library is native server code, licensed LGPL-3.0-only WITH
LGPL-3.0-linking-exception. It is not published to a registry.
