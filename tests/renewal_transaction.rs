//! Original renewal/release semantics composed in the actual owner writer.
use chrono::{DateTime, Duration, Utc};
use crgs::{
    Admission, Error, Handle, LibsqlStorage, Member, MemberId, Register, ReleasePeriod,
    Reservation, Role, SCHEMA, YearMonth,
};

fn now() -> DateTime<Utc> {
    "2026-09-30T12:00:00Z".parse().unwrap()
}
fn month(year: u16, value: u8) -> YearMonth {
    YearMonth::new(year, value).unwrap()
}
async fn fixture() -> (tempfile::TempDir, crlt::Db, Register<LibsqlStorage>, Member) {
    let directory = tempfile::tempdir().unwrap();
    let db = crlt::Db::open(crlt::Config::new(
        format!("file://{}", directory.path().join("renew.db").display()),
        "",
    ))
    .await
    .unwrap();
    db.migrate(&[crlt::Migration::new(1, "crgs", SCHEMA)])
        .await
        .unwrap();
    let register = Register::new(
        LibsqlStorage::new(db.community("scope").unwrap()),
        ReleasePeriod::months(3),
    );
    let id = MemberId::new(vec![1]).unwrap();
    let handle = Handle::from_display("original_member").unwrap();
    register
        .reserve_handle(
            Reservation {
                member_id: id.clone(),
                handle: handle.clone(),
                expires_at: now() + Duration::hours(1),
            },
            now(),
        )
        .await
        .unwrap();
    let member = register
        .admit(
            Admission {
                id,
                handle,
                role: Role::Admin,
                lease_end: month(2026, 12),
            },
            now(),
        )
        .await
        .unwrap();
    (directory, db, register, member)
}

#[tokio::test]
async fn original_renewal_commits_only_with_the_caller_and_retry_preserves_role_and_handle() {
    let (_directory, db, register, before) = fixture().await;
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    let (tx, after) = register
        .extend_lease_in_transaction(tx, &before.id, month(2027, 5), now())
        .await
        .unwrap();
    assert_eq!(
        register.member(&before.id).await.unwrap(),
        Some(before.clone())
    );
    let mut expected = before.clone();
    expected.lease_end = month(2027, 5);
    assert_eq!(after, expected);
    tx.commit().await.unwrap();
    for requested in [month(2027, 5), month(2026, 12)] {
        let tx = db.community("scope").unwrap().tx().await.unwrap();
        let (tx, retry) = register
            .extend_lease_in_transaction(tx, &before.id, requested, now())
            .await
            .unwrap();
        assert_eq!(retry, expected);
        tx.commit().await.unwrap();
    }
}

#[tokio::test]
async fn refused_later_join_drops_an_earlier_renewal_and_foreign_scope_cannot_renew() {
    let (_directory, db, register, before) = fixture().await;
    let missing = MemberId::new(vec![2]).unwrap();
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    let (tx, _) = register
        .extend_lease_in_transaction(tx, &before.id, month(2027, 5), now())
        .await
        .unwrap();
    assert!(matches!(
        register
            .extend_lease_in_transaction(tx, &missing, month(2027, 5), now())
            .await,
        Err(Error::MemberNotFound)
    ));
    assert_eq!(
        register.member(&before.id).await.unwrap(),
        Some(before.clone())
    );
    let tx = db.community("foreign").unwrap().tx().await.unwrap();
    assert!(matches!(
        register
            .extend_lease_in_transaction(tx, &before.id, month(2027, 5), now())
            .await,
        Err(Error::Storage)
    ));
    assert_eq!(register.member(&before.id).await.unwrap(), Some(before));
}

#[tokio::test]
async fn due_lazy_release_is_staged_atomically_and_never_resurrects_the_original_handle() {
    let (_directory, db, register, before) = fixture().await;
    let late = "2028-01-01T00:00:00Z".parse().unwrap();
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    let (tx, released) = register
        .extend_lease_in_transaction(tx, &before.id, month(2029, 1), late)
        .await
        .unwrap();
    assert_eq!(released.lease_end, before.lease_end);
    assert!(released.handle.is_none());
    drop(tx);
    assert_eq!(
        register.member(&before.id).await.unwrap(),
        Some(before.clone())
    );
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    let (tx, actual) = register
        .extend_lease_in_transaction(tx, &before.id, month(2029, 1), late)
        .await
        .unwrap();
    assert_eq!(actual, released);
    tx.commit().await.unwrap();
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    assert!(matches!(
        register
            .extend_lease_in_transaction(tx, &before.id, month(2029, 1), late)
            .await,
        Err(Error::LeaseExpired)
    ));
    assert_eq!(register.member(&before.id).await.unwrap(), Some(released));
}

#[tokio::test]
async fn cancellation_of_a_next_renewal_future_drops_the_previous_staged_renewal() {
    let (_directory, db, register, before) = fixture().await;
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    let (tx, _) = register
        .extend_lease_in_transaction(tx, &before.id, month(2027, 5), now())
        .await
        .unwrap();
    let pending = register.extend_lease_in_transaction(tx, &before.id, month(2028, 5), now());
    drop(pending);
    assert_eq!(register.member(&before.id).await.unwrap(), Some(before));
}
