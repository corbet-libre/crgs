//! Original lifecycle in a caller-owned writer, without caller commit forgery.
use chrono::{DateTime, Duration, Utc};
use crgs::{
    Admission, Error, Handle, LibsqlStorage, MemberId, Register, ReleasePeriod, Reservation, Role,
    SCHEMA, YearMonth,
};

fn now() -> DateTime<Utc> {
    "2026-09-30T12:00:00Z".parse().unwrap()
}
fn admission(value: u8) -> Admission {
    Admission {
        id: MemberId::new(vec![value]).unwrap(),
        handle: Handle::from_display(&format!("member_{value}")).unwrap(),
        role: Role::Root,
        lease_end: YearMonth::new(2026, 12).unwrap(),
    }
}
async fn open(path: &std::path::Path) -> crlt::Db {
    // A second connection serves the read that must not see the open writer.
    let mut config = crlt::Config::new(format!("file://{}", path.display()), "");
    config.max_connections = 2;
    let db = crlt::Db::open(config).await.unwrap();
    db.migrate(&[crlt::Migration::new(1, "crgs", SCHEMA)])
        .await
        .unwrap();
    db
}
fn register(db: &crlt::Db) -> Register<LibsqlStorage> {
    Register::new(
        LibsqlStorage::new(db.community("scope").unwrap()),
        ReleasePeriod::months(3),
    )
}
async fn reserve(register: &Register<LibsqlStorage>, admission: &Admission) {
    register
        .reserve_handle(
            Reservation {
                member_id: admission.id.clone(),
                handle: admission.handle.clone(),
                expires_at: now() + Duration::hours(1),
            },
            now(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn joined_admission_commits_only_when_caller_commits_and_exact_retry_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("joined.db");
    let db = open(&path).await;
    let register = register(&db);
    let request = admission(1);
    reserve(&register, &request).await;
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    let (tx, first) = register
        .admit_in_transaction(tx, request.clone(), now())
        .await
        .unwrap();
    assert!(register.member(&request.id).await.unwrap().is_none());
    tx.commit().await.unwrap();
    let other = open(&path).await;
    let reopened = Register::new(
        LibsqlStorage::new(other.community("scope").unwrap()),
        ReleasePeriod::months(3),
    );
    assert_eq!(
        reopened.member(&request.id).await.unwrap(),
        Some(first.clone())
    );
    let tx = other.community("scope").unwrap().tx().await.unwrap();
    let (tx, retry) = reopened
        .admit_in_transaction(tx, request, now())
        .await
        .unwrap();
    assert_eq!(retry, first);
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn later_domain_refusal_drops_all_earlier_joined_admission_changes() {
    let directory = tempfile::tempdir().unwrap();
    let db = open(&directory.path().join("rollback.db")).await;
    let register = register(&db);
    let first = admission(1);
    reserve(&register, &first).await;
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    let (tx, _) = register
        .admit_in_transaction(tx, first.clone(), now())
        .await
        .unwrap();
    assert!(matches!(
        register.admit_in_transaction(tx, admission(2), now()).await,
        Err(Error::ReservationRequired)
    ));
    assert!(register.member(&first.id).await.unwrap().is_none());
    assert!(register.reservation(&first.id).await.unwrap().is_some());
    // Original release-policy and reservation state still support real retry.
    assert_eq!(register.admit(first, now()).await.unwrap().role, Role::Root);
}

#[tokio::test]
async fn different_scope_or_separately_opened_same_url_cannot_join_this_register() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity.db");
    let db = open(&path).await;
    let register = register(&db);
    let request = admission(1);
    reserve(&register, &request).await;
    let tx = db.community("foreign").unwrap().tx().await.unwrap();
    assert!(matches!(
        register
            .admit_in_transaction(tx, request.clone(), now())
            .await,
        Err(Error::Storage)
    ));
    let other = open(&path).await;
    let tx = other.community("scope").unwrap().tx().await.unwrap();
    assert!(matches!(
        register
            .admit_in_transaction(tx, request.clone(), now())
            .await,
        Err(Error::Storage)
    ));
    assert!(register.member(&request.id).await.unwrap().is_none());
}

#[tokio::test]
async fn cancelled_next_owner_future_cannot_commit_an_earlier_staged_admission() {
    let directory = tempfile::tempdir().unwrap();
    let db = open(&directory.path().join("cancel.db")).await;
    let register = register(&db);
    let request = admission(1);
    reserve(&register, &request).await;
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    let (tx, _) = register
        .admit_in_transaction(tx, request.clone(), now())
        .await
        .unwrap();
    let pending = register.admit_in_transaction(tx, request.clone(), now());
    drop(pending);
    assert!(register.member(&request.id).await.unwrap().is_none());
    assert!(register.reservation(&request.id).await.unwrap().is_some());
}
