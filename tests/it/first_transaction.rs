//! Conservative original-register creation probes in the actual caller writer.
use chrono::{DateTime, Duration, Utc};
use crgs::{
    Admission, Error, Handle, LibsqlStorage, MemberId, Register, ReleasePeriod, Reservation, Role,
    SCHEMA, YearMonth,
};

async fn fixture() -> (
    tempfile::TempDir,
    crlt::Db,
    Register<LibsqlStorage>,
    Admission,
) {
    let directory = tempfile::tempdir().unwrap();
    let db = crlt::Db::open(crlt::Config::new(
        format!("file://{}", directory.path().join("first.db").display()),
        "",
    ))
    .await
    .unwrap();
    db.migrate(&[crlt::Migration::new(1, "crgs", SCHEMA)])
        .await
        .unwrap();
    let register = Register::new(
        LibsqlStorage::new(db.community("scope").unwrap()),
        ReleasePeriod::default(),
    );
    let admission = Admission {
        id: MemberId::new(vec![1]).unwrap(),
        handle: Handle::from_display("original_member").unwrap(),
        role: Role::Member,
        lease_end: YearMonth::new(2026, 12).unwrap(),
    };
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
    (directory, db, register, admission)
}
fn now() -> DateTime<Utc> {
    "2026-09-30T12:00:00Z".parse().unwrap()
}

#[tokio::test]
async fn pending_reservation_is_not_a_member_but_a_retained_legacy_member_is_not_fresh_creation() {
    let (_directory, db, register, admission) = fixture().await;
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    let tx = register.ensure_empty_in_transaction(tx).await.unwrap();
    tx.commit().await.unwrap();
    let original = register.admit(admission.clone(), now()).await.unwrap();
    register
        .release_expired("2040-01-01T00:00:00Z".parse().unwrap(), 10)
        .await
        .unwrap();
    let mut released = original;
    released.handle = None;
    // Even well after expiry/release, any original row prevents adopting the
    // existing scoped Register as a newly created community.
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    assert!(matches!(
        register.ensure_empty_in_transaction(tx).await,
        Err(Error::MemberExists)
    ));
    assert_eq!(
        register.member(&admission.id).await.unwrap(),
        Some(released)
    );
}

#[tokio::test]
async fn refusing_nonempty_initialization_drops_prior_staging_in_the_same_writer() {
    let (_directory, db, register, admission) = fixture().await;
    let tx = db.community("scope").unwrap().tx().await.unwrap();
    let (tx, _) = register
        .admit_in_transaction(tx, admission.clone(), now())
        .await
        .unwrap();
    assert!(matches!(
        register.ensure_empty_in_transaction(tx).await,
        Err(Error::MemberExists)
    ));
    assert!(register.member(&admission.id).await.unwrap().is_none());
    assert!(register.reservation(&admission.id).await.unwrap().is_some());
    assert_eq!(
        register.admit(admission, now()).await.unwrap().role,
        Role::Member
    );
}

#[tokio::test]
async fn foreign_scope_cannot_supply_an_empty_register_assertion() {
    let (_directory, db, register, admission) = fixture().await;
    let tx = db.community("foreign").unwrap().tx().await.unwrap();
    assert!(matches!(
        register.ensure_empty_in_transaction(tx).await,
        Err(Error::Storage)
    ));
    assert!(register.member(&admission.id).await.unwrap().is_none());
}
