//! The joined read is storage data, never a Root or admission capability.
use chrono::{DateTime, Duration, Utc};
use crgs::{
    Admission, Error, Handle, LibsqlStorage, MemberId, Register, ReleasePeriod, Reservation, Role,
    SCHEMA, YearMonth,
};

fn now() -> DateTime<Utc> {
    "2026-09-30T12:00:00Z".parse().unwrap()
}
async fn fixture() -> (tempfile::TempDir, crlt::Db, Register<LibsqlStorage>) {
    let directory = tempfile::tempdir().unwrap();
    let db = crlt::Db::open(crlt::Config::new(
        format!("file://{}", directory.path().join("read.db").display()),
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
    (directory, db, register)
}
fn id(value: u8) -> MemberId {
    MemberId::new(vec![value]).unwrap()
}

#[tokio::test]
async fn missing_joined_read_commits_no_member_or_role() {
    let (_directory, db, register) = fixture().await;
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    let (tx, row) = register.member_in_transaction(tx, &id(1)).await.unwrap();
    assert!(row.is_none());
    tx.commit().await.unwrap();
    assert!(register.member(&id(1)).await.unwrap().is_none());
}

#[tokio::test]
async fn joined_read_uses_exact_current_staging_and_cancellation_rolls_it_back() {
    let (_directory, db, register) = fixture().await;
    let handle = Handle::from_display("original_member").unwrap();
    register
        .reserve_handle(
            Reservation {
                member_id: id(1),
                handle: handle.clone(),
                expires_at: now() + Duration::hours(1),
            },
            now(),
        )
        .await
        .unwrap();
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    let (tx, staged) = register
        .admit_in_transaction(
            tx,
            Admission {
                id: id(1),
                handle,
                role: Role::Admin,
                lease_end: YearMonth::new(2026, 12).unwrap(),
            },
            now(),
        )
        .await
        .unwrap();
    let (tx, row) = register.member_in_transaction(tx, &id(1)).await.unwrap();
    assert_eq!(row, Some(staged));
    assert!(register.member(&id(1)).await.unwrap().is_none());
    let owned_id = id(1);
    let pending = register.member_in_transaction(tx, &owned_id);
    drop(pending);
    assert!(register.member(&id(1)).await.unwrap().is_none());
    assert!(register.reservation(&id(1)).await.unwrap().is_some());
}

#[tokio::test]
async fn foreign_scope_or_independent_same_url_database_handle_cannot_supply_member_data() {
    let (directory, db, register) = fixture().await;
    let tx = db.community("foreign").unwrap().tx().await.unwrap();
    assert!(matches!(
        register.member_in_transaction(tx, &id(1)).await,
        Err(Error::Storage)
    ));
    let other = crlt::Db::open(crlt::Config::new(
        format!("file://{}", directory.path().join("read.db").display()),
        "",
    ))
    .await
    .unwrap();
    other
        .migrate(&[crlt::Migration::new(1, "crgs", SCHEMA)])
        .await
        .unwrap();
    let tx = other.community("scope").unwrap().tx().await.unwrap();
    assert!(matches!(
        register.member_in_transaction(tx, &id(1)).await,
        Err(Error::Storage)
    ));
    assert!(register.member(&id(1)).await.unwrap().is_none());
}
