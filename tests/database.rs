//! Persistent, concurrent, scoped and optional remote database round trips.

use chrono::{DateTime, Duration, Utc};
use crgs::{
    Admission, Handle, LibsqlStorage, MemberId, Register, ReleasePeriod, Reservation, Role, SCHEMA,
    Storage, Transaction, YearMonth,
};
use crlt::{Config, Db, Migration};

fn now() -> DateTime<Utc> {
    "2026-09-30T12:00:00Z".parse().unwrap()
}
fn id(value: u8) -> MemberId {
    MemberId::new(vec![0, value, 255]).unwrap()
}
fn handle(name: &str) -> Handle {
    Handle::from_display(name).unwrap()
}
fn register(db: &Db, key: &str) -> Register<LibsqlStorage> {
    Register::new(
        LibsqlStorage::new(db.community(key).unwrap()),
        ReleasePeriod::default(),
    )
}
async fn open(url: &str) -> Db {
    let db = Db::open(Config::new(url, "")).await.unwrap();
    db.migrate(&[Migration::new(1, "crgs", SCHEMA)])
        .await
        .unwrap();
    db
}
async fn enroll(register: &Register<LibsqlStorage>, value: u8, name: &str, role: Role) {
    register
        .reserve_handle(
            Reservation {
                member_id: id(value),
                handle: handle(name),
                expires_at: now() + Duration::hours(1),
            },
            now(),
        )
        .await
        .unwrap();
    register
        .admit(
            Admission {
                id: id(value),
                handle: handle(name),
                role,
                lease_end: YearMonth::new(2026, 9).unwrap(),
            },
            now(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn reopen_preserves_members_reservations_blocks_and_migration_history() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "file://{}",
        directory.path().join("persistent.db").display()
    );
    {
        let db = open(&url).await;
        let register = register(&db, "community");
        enroll(&register, 1, "original_handle", Role::Admin).await;
        register
            .change_handle(&id(1), handle("current_handle"), now())
            .await
            .unwrap();
        register
            .reserve_handle(
                Reservation {
                    member_id: id(2),
                    handle: handle("pending_handle"),
                    expires_at: now() + Duration::hours(1),
                },
                now(),
            )
            .await
            .unwrap();
    }
    let db = open(&url).await;
    let register = register(&db, "community");
    let member = register.member(&id(1)).await.unwrap().unwrap();
    assert_eq!(member.role, Role::Admin);
    assert_eq!(member.handle.unwrap(), handle("current_handle"));
    assert!(
        !register
            .is_handle_available(handle("original_handle").skeleton(), now())
            .await
            .unwrap()
    );
    assert!(
        !register
            .is_handle_available(handle("pending_handle").skeleton(), now())
            .await
            .unwrap()
    );
    register
        .admit(
            Admission {
                id: id(2),
                handle: handle("pending_handle"),
                role: Role::Member,
                lease_end: YearMonth::new(2026, 12).unwrap(),
            },
            now(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn every_operation_is_scoped_even_when_a_database_is_shared() {
    let directory = tempfile::tempdir().unwrap();
    let db = open(&format!(
        "file://{}",
        directory.path().join("isolation.db").display()
    ))
    .await;
    let a = register(&db, "community-a' OR 1=1 --");
    let b = register(&db, "community-b");
    enroll(&a, 1, "shared_name", Role::Root).await;
    assert!(b.member(&id(1)).await.unwrap().is_none());
    assert!(
        b.is_handle_available(handle("shared_name").skeleton(), now())
            .await
            .unwrap()
    );
    enroll(&b, 1, "shared_name", Role::Member).await;
    a.set_role(&id(1), Role::Admin).await.unwrap();
    a.extend_lease(&id(1), YearMonth::new(2030, 1).unwrap(), now())
        .await
        .unwrap();
    a.change_handle(&id(1), handle("new_name"), now())
        .await
        .unwrap();
    assert_eq!(
        b.by_handle(handle("shared_name").skeleton(), now())
            .await
            .unwrap()
            .unwrap()
            .role,
        Role::Member
    );
    assert_eq!(
        b.member(&id(1)).await.unwrap().unwrap().lease_end,
        YearMonth::new(2026, 9).unwrap()
    );
    let pending = Reservation {
        member_id: id(2),
        handle: handle("same_pending"),
        expires_at: now() + Duration::hours(1),
    };
    a.reserve_handle(pending.clone(), now()).await.unwrap();
    b.reserve_handle(pending, now()).await.unwrap();
    assert!(
        a.cancel_reservation(&id(2), handle("same_pending").skeleton())
            .await
            .unwrap()
    );
    assert!(
        !b.is_handle_available(handle("same_pending").skeleton(), now())
            .await
            .unwrap()
    );
    let later = "2028-10-01T00:00:00Z".parse().unwrap();
    let released = b.release_expired(later, 10).await.unwrap();
    assert_eq!(released.handles, 1);
    assert_eq!(released.reservations, 1);
    assert_eq!(released.blocks, 0);
    assert!(
        a.by_handle(handle("new_name").skeleton(), later)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(a.release_expired(later, 10).await.unwrap().blocks, 1);
    assert!(b.member(&id(1)).await.unwrap().unwrap().handle.is_none());
}

#[tokio::test]
async fn independent_database_handles_cannot_double_claim() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!("file://{}", directory.path().join("race.db").display());
    let a = register(&open(&url).await, "same_community");
    let b = register(&open(&url).await, "same_community");
    let first = Reservation {
        member_id: id(1),
        handle: handle("racing_handle"),
        expires_at: now() + Duration::hours(1),
    };
    let second = Reservation {
        member_id: id(2),
        ..first.clone()
    };
    let (left, right) = tokio::join!(
        a.reserve_handle(first, now()),
        b.reserve_handle(second, now())
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert!(matches!(
        left.as_ref().err().or(right.as_ref().err()),
        Some(crgs::Error::HandleUnavailable | crgs::Error::Storage)
    ));
    let winner = left.ok().or(right.ok()).unwrap();
    let member = a
        .admit(
            Admission {
                id: winner.member_id,
                handle: winner.handle,
                role: Role::Member,
                lease_end: YearMonth::new(2026, 12).unwrap(),
            },
            now(),
        )
        .await
        .unwrap();
    assert_eq!(
        b.by_handle(handle("racing_handle").skeleton(), now())
            .await
            .unwrap(),
        Some(member)
    );
}

#[tokio::test]
async fn cancelled_transaction_releases_connection_and_rolls_back() {
    let directory = tempfile::tempdir().unwrap();
    let db = open(&format!(
        "file://{}",
        directory.path().join("cancellation.db").display()
    ))
    .await;
    let storage = LibsqlStorage::new(db.community("cancelled").unwrap());
    let worker_storage = storage.clone();
    let (written, received) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut tx = worker_storage.begin().await.unwrap();
        tx.save_claim(
            handle("uncommitted").skeleton(),
            &crgs::storage::Claim::Blocked(YearMonth::new(2028, 10).unwrap()),
        )
        .await
        .unwrap();
        written.send(()).unwrap();
        std::future::pending::<()>().await;
        tx.commit().await.unwrap();
    });
    received.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let mut tx = storage.begin().await.unwrap();
    assert_eq!(
        tx.claim(handle("uncommitted").skeleton()).await.unwrap(),
        None
    );
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn optional_turso_round_trip() {
    let (Ok(url), Ok(token)) = (std::env::var("TURSO_URL"), std::env::var("TURSO_TOKEN")) else {
        eprintln!("skipped Turso round trip: both environment variables are required");
        return;
    };
    if url.is_empty() || token.is_empty() {
        eprintln!("skipped Turso round trip: both environment variables must be nonempty");
        return;
    }
    // The supplied database must be disposable. Never log either configuration value.
    let db = Db::open(Config::new(url, token)).await.unwrap();
    db.migrate(&[Migration::new(1, "crgs", SCHEMA)])
        .await
        .unwrap();
    let namespace = format!(
        "crgs-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let scope = db.community(namespace).unwrap();
    let register = Register::new(LibsqlStorage::new(scope.clone()), ReleasePeriod::default());
    enroll(&register, 1, "turso_test_member", Role::Member).await;
    assert_eq!(register.role(&id(1)).await.unwrap(), Some(Role::Member));
    register
        .change_handle(&id(1), handle("turso_changed_member"), now())
        .await
        .unwrap();
    let later = "2028-10-01T00:00:00Z".parse().unwrap();
    let released = register.release_expired(later, 10).await.unwrap();
    assert_eq!(released.handles, 1);
    assert_eq!(released.blocks, 1);
    assert!(
        register
            .is_handle_available(handle("turso_changed_member").skeleton(), later)
            .await
            .unwrap()
    );
    scope
        .execute(
            "DELETE FROM crgs_members WHERE member_id = ?1",
            crlt::params![id(1).as_bytes().to_vec()],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn advisory_reads_do_not_wait_for_an_unrelated_write_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::new(
        format!("file://{}", dir.path().join("reads.db").display()),
        "",
    );
    config.max_connections = 2;
    let db = Db::open(config).await.unwrap();
    db.migrate(&[Migration::new(1, "crgs", SCHEMA)])
        .await
        .unwrap();
    let r = register(&db, "a");
    enroll(&r, 1, "first", Role::Member).await;
    let storage = LibsqlStorage::new(db.community("a").unwrap());
    let writer = storage.begin().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        assert!(r.member(&id(1)).await.unwrap().is_some());
        assert!(
            r.by_handle(handle("first").skeleton(), now())
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            r.is_handle_available(handle("second").skeleton(), now())
                .await
                .unwrap()
        );
        assert!(r.reservation(&id(2)).await.unwrap().is_none());
    })
    .await
    .expect("reads must not request the writer lock");
    drop(writer);
    let mut read = storage.begin_read().await.unwrap();
    assert!(read.remove_claim("first").await.is_err());
    assert!(
        r.by_handle(handle("first").skeleton(), now())
            .await
            .unwrap()
            .is_some()
    );
}

async fn read_capability_refuses_mutation<S: Storage>(storage: S) {
    use crgs::storage::{Claim, StoredMember};
    let member = StoredMember {
        member: crgs::Member {
            id: id(1),
            handle: Some(handle("existing")),
            role: Role::Member,
            lease_end: YearMonth::new(2026, 9).unwrap(),
        },
        release_on: Some(YearMonth::new(2028, 10).unwrap()),
    };
    let mut writer = storage.begin().await.unwrap();
    writer.save_member(&member).await.unwrap();
    writer.save_claim("existing", &Claim::Active(id(1))).await.unwrap();
    writer.commit().await.unwrap();
    let mut read = storage.begin_read().await.unwrap();
    let mut changed = member.clone();
    changed.member.role = Role::Root;
    assert_eq!(read.save_member(&changed).await, Err(crgs::Error::Storage));
    assert_eq!(read.save_claim("other", &Claim::Active(id(2))).await, Err(crgs::Error::Storage));
    assert_eq!(read.remove_claim("existing").await, Err(crgs::Error::Storage));
    drop(read);
    let mut verify = storage.begin_read().await.unwrap();
    assert_eq!(verify.member(&id(1)).await.unwrap(), Some(member));
    assert_eq!(verify.claim("existing").await.unwrap(), Some(Claim::Active(id(1))));
    assert_eq!(verify.claim("other").await.unwrap(), None);
    verify.commit().await.unwrap();
}

#[tokio::test]
async fn memory_and_sql_read_capabilities_refuse_all_mutations() {
    read_capability_refuses_mutation(crgs::MemoryStorage::default()).await;
    let directory = tempfile::tempdir().unwrap();
    let url = format!("file://{}", directory.path().join("read-only.db").display());
    let db = open(&url).await;
    read_capability_refuses_mutation(LibsqlStorage::new(db.community("community").unwrap())).await;
}

#[tokio::test]
async fn malformed_stored_handle_prevents_transaction_commit() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!("file://{}", directory.path().join("corrupt-handle.db").display());
    let db = open(&url).await;
    let register = register(&db, "community");
    enroll(&register, 1, "original", Role::Member).await;
    let scope = db.community("community").unwrap();
    // Simulate a damaged import without bypassing the real SQL constraints.
    scope
        .execute(
            "UPDATE crgs_members SET skeleton = ?1 WHERE member_id = ?2",
            crlt::params!["different", id(1).as_bytes().to_vec()],
        )
        .await
        .unwrap();
    let storage = LibsqlStorage::new(scope);
    let mut tx = storage.begin().await.unwrap();
    assert_eq!(tx.member(&id(1)).await, Err(crgs::Error::Storage));
    assert_eq!(tx.commit().await, Err(crgs::Error::Storage));
    assert_eq!(register.member(&id(1)).await, Err(crgs::Error::Storage));
}

#[tokio::test]
async fn expired_admission_and_invalid_lookup_preserve_live_reservation() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!("file://{}", directory.path().join("expired-admission.db").display());
    let db = open(&url).await;
    let register = register(&db, "community");
    let reservation = Reservation {
        member_id: id(1),
        handle: handle("pending"),
        expires_at: now() + Duration::hours(1),
    };
    register.reserve_handle(reservation.clone(), now()).await.unwrap();
    assert_eq!(
        register
            .admit(
                Admission {
                    id: id(1),
                    handle: handle("pending"),
                    role: Role::Member,
                    lease_end: YearMonth::new(2026, 8).unwrap(),
                },
                now(),
            )
            .await,
        Err(crgs::Error::LeaseExpired)
    );
    assert_eq!(register.reservation(&id(1)).await.unwrap(), Some(reservation));
    assert_eq!(register.member(&id(1)).await.unwrap(), None);
    assert_eq!(register.is_handle_available("pending\0", now()).await, Err(crgs::Error::InvalidInput));
}
