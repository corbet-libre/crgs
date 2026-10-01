# crgs

**Community register: pseudonyms, handles, roles and coarse leases.**

Part of `cvld`, the permanent door of the cmtymeet trust stack. Native Rust;
interfaces may change. See [the implemented contract](docs/CONTRACT.md) for
boundaries, expiry semantics and caller responsibilities.

## Dependency survey

Surveyed crates.io and GitHub on 2026-09-30 before implementation:

| Candidate | Choice and reason |
|---|---|
| [crlt](https://github.com/corbet-foss/crlt) (LGPL-3.0-only WITH linking exception) | Selected git dependency following main with a locked revision: it exposes scoped transactions, migrations and query-plan enforcement. The service supplies a fixed community capability. |
| [libsql 0.9.30](https://crates.io/crates/libsql) ([upstream](https://github.com/tursodatabase/libsql), MIT) | Official local/Turso client, used through crlt. No direct driver dependency or temporary migration adapter is needed. |
| [SQLx](https://github.com/launchbadge/sqlx) (MIT/Apache-2.0) | Maintained SQL toolkit; its SQLite adapter would not supply the required Turso and crlt community boundary. An ORM adds no value for this small schema. |
| [unicode-security](https://github.com/unicode-rs/unicode-security) (MIT/Apache-2.0) | Selected with precis-profiles to verify skeleton/display binding, using the same pinned versions as cgrd. |
| [chrono](https://github.com/chronotope/chrono) (MIT/Apache-2.0) | UTC calendar conversion from caller-supplied times; clock and timezone discovery features are disabled. |

Tokio supplies in-memory transaction locking; `thiserror` supplies fixed typed
errors. `tempfile` is test-only. These dependencies are permissively licensed.
Register-specific lifecycle rules stay here; SQL, transactions, pooling, TLS,
and Unicode validation remain with their respective established libraries.
No other cvld crate API is required: the pseudonym is opaque and handle
validation is the caller's responsibility.

## API

`Register<S>` works with `MemoryStorage` or `LibsqlStorage`. Memory storage is
intended for tests and copies its small state per transaction; use libSQL for
persistent deployments. Operations include
`reserve_handle`, `cancel_reservation`, `admit`, `member`, `by_handle`,
`is_handle_available`, `role`, `set_role`, `extend_lease`, `change_handle`, and
`release_expired`. All mutations are atomic. Release policy defaults to 24
months; leases and block deadlines contain only year and month. Reservation
expiry is transient and removed when no longer needed.

```rust
use crgs::{Admission, Handle, MemberId, MemoryStorage, Register,
    ReleasePeriod, Reservation, Role, YearMonth};
use chrono::{DateTime, Duration, Utc};

# async fn example() -> crgs::Result<()> {
let register = Register::new(MemoryStorage::default(), ReleasePeriod::default());
let now: DateTime<Utc> = "2026-09-30T12:00:00Z".parse().unwrap();
let id = MemberId::new(vec![0, 255, 42])?;
let handle = Handle::from_display("alice_handle")?;
register.reserve_handle(Reservation {
    member_id: id.clone(), handle: handle.clone(),
    expires_at: now + Duration::hours(1),
}, now).await?;
register.admit(Admission {
    id, handle, role: Role::Member, lease_end: YearMonth::new(2027, 9)?,
}, now).await?;
# Ok(())
# }
```

The skeleton is derived from the normalized display. `Handle::new` also accepts
a supplied skeleton but verifies it before use. Production callers supply
the trusted current UTC time; crgs never reads a system clock itself.

The service adds `SCHEMA` to its full `crlt` migration history and passes a
`crlt::Community` to `LibsqlStorage::new`. One database per community is a
service routing rule; SQL namespace checks provide an additional boundary.
Never enable dependency SQL/HTTP debug tracing in a deployment.

## Verification

GitHub Actions runs `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and `cargo test` on stable Rust.
The committed `Cargo.lock` and CI artifact retain the tested dependency graph.
Do not run Cargo on the workstation. Tests exercise the same register API
against real memory transactions and temporary libSQL files, plus persistence,
concurrency, isolation, query plans and failures.

The optional Turso test returns without connecting unless **both** `TURSO_URL`
and `TURSO_TOKEN` are nonempty. Supply a disposable test database from a trusted
test runner: the test applies schema and writes test records. No credentials
are configured in the repository or GitHub Actions. Never publish this crate
to a registry.

## License

Copyright 2026 Julian Y. Richard Corbet. Licensed under [LGPL-3.0-only](LICENSES/LGPL-3.0-only.txt) [WITH LGPL-3.0-linking-exception](LICENSES/LGPL-3.0-linking-exception.txt). See [LICENSE.md](LICENSE.md).

## Continuous verification

Dependency updates follow main and are tested against one CI-resolved lockfile.
Line and branch coverage target 100%; failures remain blocking. See
[the coverage contract](docs/COVERAGE.md) for measurement and exclusions.
