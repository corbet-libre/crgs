//! Shared lifecycle contracts against memory and real libSQL storage.

use chrono::{DateTime, Duration, Utc};
use crgs::{
    Admission, Error, Handle, LibsqlStorage, MemberId, MemoryStorage, Register, ReleasePeriod,
    Released, Reservation, Role, SCHEMA, Storage, Transaction, YearMonth, storage::Claim,
};
use crlt::{Config, Db, Migration};

fn time(value: &str) -> DateTime<Utc> {
    value.parse().unwrap()
}
fn now() -> DateTime<Utc> {
    time("2026-09-30T12:00:00Z")
}
fn month(year: u16, month: u8) -> YearMonth {
    YearMonth::new(year, month).unwrap()
}
fn id(value: u8) -> MemberId {
    MemberId::new(vec![0, 255, value]).unwrap()
}
fn handle(value: &str) -> Handle {
    Handle::from_display(value).unwrap()
}
fn reservation(value: u8, name: &str) -> Reservation {
    Reservation {
        member_id: id(value),
        handle: handle(name),
        expires_at: now() + Duration::hours(1),
    }
}
async fn enroll<S: Storage>(register: &Register<S>, value: u8, name: &str, lease: YearMonth) {
    register
        .reserve_handle(reservation(value, name), now())
        .await
        .unwrap();
    register
        .admit(
            Admission {
                id: id(value),
                handle: handle(name),
                role: Role::Member,
                lease_end: lease,
            },
            now(),
        )
        .await
        .unwrap();
}
async fn memory() -> (Option<tempfile::TempDir>, MemoryStorage) {
    (None, MemoryStorage::default())
}
async fn sql() -> (Option<tempfile::TempDir>, LibsqlStorage) {
    let directory = tempfile::tempdir().unwrap();
    let db = Db::open(Config::new(
        format!("file://{}", directory.path().join("register.db").display()),
        "",
    ))
    .await
    .unwrap();
    db.migrate(&[Migration::new(1, "crgs", SCHEMA)])
        .await
        .unwrap();
    (
        Some(directory),
        LibsqlStorage::new(db.community("community-a").unwrap()),
    )
}

async fn lifecycle<S: Storage + Clone>(storage: S) {
    let register = Register::new(storage, ReleasePeriod::default());
    assert_eq!(register.member(&id(1)).await.unwrap(), None);
    assert_eq!(register.role(&id(1)).await.unwrap(), None);
    let r = reservation(1, "member_one");
    assert_eq!(register.reserve_handle(r.clone(), now()).await.unwrap(), r);
    assert_eq!(
        register
            .reserve_handle(r.clone(), now() + Duration::minutes(5))
            .await
            .unwrap(),
        r
    );
    assert_eq!(
        register
            .by_handle(handle("member_one").skeleton(), now())
            .await
            .unwrap(),
        None
    );
    let a = Admission {
        id: id(1),
        handle: handle("member_one"),
        role: Role::Root,
        lease_end: month(2026, 12),
    };
    let member = register.admit(a.clone(), now()).await.unwrap();
    assert_eq!(register.admit(a, now()).await.unwrap(), member);
    assert_eq!(
        register
            .by_handle(handle("member_one").skeleton(), now())
            .await
            .unwrap(),
        Some(member.clone())
    );
    assert_eq!(register.role(&id(1)).await.unwrap(), Some(Role::Root));
    assert_eq!(
        register.set_role(&id(1), Role::Admin).await.unwrap().role,
        Role::Admin
    );
    assert_eq!(
        register.set_role(&id(1), Role::Member).await.unwrap().role,
        Role::Member
    );
    assert_eq!(
        register
            .extend_lease(&id(1), month(2027, 2), now())
            .await
            .unwrap()
            .lease_end,
        month(2027, 2)
    );
    assert_eq!(
        register
            .extend_lease(&id(1), month(2026, 1), now())
            .await
            .unwrap()
            .lease_end,
        month(2027, 2)
    );
    assert_eq!(
        register
            .reserve_handle(reservation(1, "second_name"), now())
            .await,
        Err(Error::MemberExists)
    );
}

async fn uniqueness<S: Storage + Clone>(storage: S) {
    let register = Register::new(storage, ReleasePeriod::default());
    let first = Reservation {
        member_id: id(1),
        handle: handle("paypalname"),
        expires_at: now() + Duration::hours(1),
    };
    register.reserve_handle(first.clone(), now()).await.unwrap();
    let mut confusable = first.clone();
    confusable.member_id = id(2);
    confusable.handle = handle("pаypalname"); // Cyrillic a; the library derives the collision.
    assert_eq!(
        register.reserve_handle(confusable, now()).await,
        Err(Error::HandleUnavailable)
    );
    let mut changed_expiry = first.clone();
    changed_expiry.expires_at += Duration::seconds(1);
    assert_eq!(
        register.reserve_handle(changed_expiry, now()).await,
        Err(Error::ReservationConflict)
    );
    assert_eq!(
        register
            .reserve_handle(reservation(1, "another_name"), now())
            .await,
        Err(Error::ReservationConflict)
    );
    let mut a = Admission {
        id: id(2),
        handle: first.handle.clone(),
        role: Role::Member,
        lease_end: month(2026, 10),
    };
    assert_eq!(
        register.admit(a.clone(), now()).await,
        Err(Error::ReservationRequired)
    );
    a.id = id(1);
    a.handle = handle("PAYPALNAME");
    assert_eq!(
        register.admit(a.clone(), now()).await,
        Err(Error::ReservationRequired)
    );
    a.handle = first.handle;
    register.admit(a.clone(), now()).await.unwrap();
    a.role = Role::Admin;
    assert_eq!(register.admit(a, now()).await, Err(Error::MemberExists));
    assert_eq!(register.role(&id(1)).await.unwrap(), Some(Role::Member));
}

async fn reservation_expiry<S: Storage + Clone>(storage: S) {
    let register = Register::new(storage.clone(), ReleasePeriod::default());
    let r = reservation(1, "expiring_name");
    let expiry = r.expires_at;
    register.reserve_handle(r.clone(), now()).await.unwrap();
    assert!(
        !register
            .is_handle_available(
                handle("expiring_name").skeleton(),
                expiry - Duration::nanoseconds(1)
            )
            .await
            .unwrap()
    );
    assert!(
        register
            .is_handle_available(handle("expiring_name").skeleton(), expiry)
            .await
            .unwrap()
    );
    let mut tx = storage.begin().await.unwrap();
    assert!(
        tx.claim(handle("expiring_name").skeleton())
            .await
            .unwrap()
            .is_some()
    ); // Reads do not clean up.
    tx.commit().await.unwrap();
    assert_eq!(
        register
            .admit(
                Admission {
                    id: id(1),
                    handle: r.handle,
                    role: Role::Member,
                    lease_end: month(2026, 12)
                },
                expiry
            )
            .await,
        Err(Error::ReservationRequired)
    );
    let replacement = Reservation {
        member_id: id(2),
        handle: handle("expiring_name"),
        expires_at: expiry + Duration::hours(1),
    };
    register.reserve_handle(replacement, expiry).await.unwrap();
    assert!(
        !register
            .cancel_reservation(&id(1), handle("expiring_name").skeleton())
            .await
            .unwrap()
    );
    assert!(
        register
            .cancel_reservation(&id(2), handle("expiring_name").skeleton())
            .await
            .unwrap()
    );
    assert!(
        !register
            .cancel_reservation(&id(2), handle("expiring_name").skeleton())
            .await
            .unwrap()
    );
    assert!(
        register
            .is_handle_available(handle("expiring_name").skeleton(), expiry)
            .await
            .unwrap()
    );
    register
        .reserve_handle(reservation(3, "other_pending"), now())
        .await
        .unwrap();
    let new = Reservation {
        member_id: id(3),
        handle: handle("new_pending"),
        expires_at: expiry + Duration::hours(1),
    };
    register.reserve_handle(new, expiry).await.unwrap();
    assert!(
        register
            .is_handle_available(handle("other_pending").skeleton(), expiry)
            .await
            .unwrap()
    );
}

async fn release_boundaries<S: Storage + Clone>(storage: S) {
    let register = Register::new(storage.clone(), ReleasePeriod::default());
    enroll(&register, 1, "lease_name", month(2026, 9)).await;
    let before = time("2028-09-30T23:59:59.999999999Z");
    let due = time("2028-10-01T00:00:00Z");
    assert_eq!(
        register.release_expired(before, 100).await.unwrap(),
        Released::default()
    );
    assert!(
        register
            .by_handle(handle("lease_name").skeleton(), before)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        register
            .by_handle(handle("lease_name").skeleton(), due)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        register
            .is_handle_available(handle("lease_name").skeleton(), due)
            .await
            .unwrap()
    );
    let released = register.release_expired(due, 100).await.unwrap();
    assert_eq!(released.handles, 1);
    let member = register.member(&id(1)).await.unwrap().unwrap();
    assert_eq!(member.handle, None);
    assert_eq!(member.role, Role::Member);
    assert_eq!(member.lease_end, month(2026, 9));
    assert_eq!(
        register.release_expired(due, 100).await.unwrap(),
        Released::default()
    );
    assert_eq!(
        register.extend_lease(&id(1), month(2028, 12), due).await,
        Err(Error::LeaseExpired)
    );
    assert_eq!(
        register
            .change_handle(&id(1), handle("new_lease_name"), due)
            .await,
        Err(Error::LeaseExpired)
    );
    assert_eq!(register.member(&id(1)).await.unwrap(), Some(member));
    let claim = Reservation {
        member_id: id(2),
        handle: handle("lease_name"),
        expires_at: due + Duration::hours(1),
    };
    register.reserve_handle(claim, due).await.unwrap();
}

async fn lazy_release_and_renewal<S: Storage + Clone>(storage: S) {
    let register = Register::new(storage, ReleasePeriod::months(0));
    enroll(&register, 1, "old_member", month(2026, 9)).await;
    enroll(&register, 2, "renewed_member", month(2026, 9)).await;
    let due = time("2026-10-01T00:00:00Z");
    let r = Reservation {
        member_id: id(3),
        handle: handle("old_member"),
        expires_at: due + Duration::hours(1),
    };
    register.reserve_handle(r, due).await.unwrap();
    assert_eq!(register.member(&id(1)).await.unwrap().unwrap().handle, None);
    let renewed = register
        .extend_lease(&id(2), month(2026, 12), due)
        .await
        .unwrap();
    assert_eq!(renewed.handle, None);
    assert_eq!(renewed.lease_end, month(2026, 9));
    for member in [1, 2] {
        assert_eq!(
            register
                .extend_lease(&id(member), month(2026, 12), due)
                .await,
            Err(Error::LeaseExpired)
        );
        assert_eq!(
            register
                .change_handle(&id(member), handle("revived_name"), due)
                .await,
            Err(Error::LeaseExpired)
        );
    }
    assert!(
        register
            .is_handle_available(handle("renewed_member").skeleton(), due)
            .await
            .unwrap()
    );
    assert_eq!(
        register.release_expired(due, 10).await.unwrap(),
        Released::default()
    );
}

async fn handle_changes<S: Storage + Clone>(storage: S) {
    let register = Register::new(storage.clone(), ReleasePeriod::default());
    enroll(&register, 1, "first_name", month(2030, 1)).await;
    enroll(&register, 2, "occupied_name", month(2030, 1)).await;
    assert_eq!(
        register
            .change_handle(&id(1), handle("occupied_name"), now())
            .await,
        Err(Error::HandleUnavailable)
    );
    assert_eq!(
        register
            .by_handle(handle("first_name").skeleton(), now())
            .await
            .unwrap()
            .unwrap()
            .id,
        id(1)
    );
    register
        .reserve_handle(reservation(3, "pending_name"), now())
        .await
        .unwrap();
    assert_eq!(
        register
            .change_handle(&id(1), handle("pending_name"), now())
            .await,
        Err(Error::HandleUnavailable)
    );
    register
        .change_handle(&id(1), handle("FIRST_NAME"), now())
        .await
        .unwrap();
    register
        .change_handle(&id(1), handle("second_name"), now())
        .await
        .unwrap();
    let later = time("2027-01-01T00:00:00Z");
    register
        .change_handle(&id(1), handle("second_name"), later)
        .await
        .unwrap();
    assert_eq!(
        register
            .change_handle(&id(1), handle("first_name"), later)
            .await,
        Err(Error::HandleUnavailable)
    );
    let mut tx = storage.begin().await.unwrap();
    assert_eq!(
        tx.claim(handle("first_name").skeleton()).await.unwrap(),
        Some(Claim::Blocked(month(2028, 10)))
    );
    tx.commit().await.unwrap();
    assert!(
        !register
            .is_handle_available(
                handle("first_name").skeleton(),
                time("2028-09-30T23:59:59Z")
            )
            .await
            .unwrap()
    );
    let due = time("2028-10-01T00:00:00Z");
    assert!(
        register
            .is_handle_available(handle("first_name").skeleton(), due)
            .await
            .unwrap()
    );
    register
        .change_handle(&id(2), handle("first_name"), due)
        .await
        .unwrap();
    assert_eq!(
        register
            .by_handle(handle("first_name").skeleton(), due)
            .await
            .unwrap()
            .unwrap()
            .id,
        id(2)
    );
}

async fn policy_deadlines<S: Storage + Clone>(storage: S) {
    let register = Register::new(storage.clone(), ReleasePeriod::months(36));
    enroll(&register, 1, "long_retention", month(2026, 9)).await;
    register
        .change_handle(&id(1), handle("current_name"), now())
        .await
        .unwrap();
    let shorter = Register::new(storage.clone(), ReleasePeriod::months(0));
    shorter
        .extend_lease(&id(1), month(2026, 10), now())
        .await
        .unwrap();
    let mut tx = storage.begin().await.unwrap();
    assert_eq!(
        tx.member(&id(1)).await.unwrap().unwrap().release_on,
        Some(month(2029, 10))
    );
    assert_eq!(
        tx.claim(handle("long_retention").skeleton()).await.unwrap(),
        Some(Claim::Blocked(month(2029, 10)))
    );
    tx.commit().await.unwrap();
    assert_eq!(
        shorter
            .release_expired(time("2026-11-01T00:00:00Z"), 10)
            .await
            .unwrap(),
        Released::default()
    );
}

async fn bounded_cleanup<S: Storage + Clone>(storage: S) {
    let register = Register::new(storage, ReleasePeriod::months(0));
    for value in 1..=3 {
        enroll(&register, value, &format!("old_{value}"), month(2026, 9)).await;
        register
            .change_handle(&id(value), handle(&format!("new_{value}")), now())
            .await
            .unwrap();
        register
            .reserve_handle(reservation(value + 10, &format!("pending_{value}")), now())
            .await
            .unwrap();
    }
    let due = time("2026-10-01T00:00:00Z");
    for _ in 0..3 {
        assert_eq!(
            register.release_expired(due, 1).await.unwrap(),
            Released {
                reservations: 1,
                handles: 1,
                blocks: 1
            }
        );
    }
    assert_eq!(
        register.release_expired(due, 1).await.unwrap(),
        Released::default()
    );
    assert_eq!(
        register.release_expired(due, 0).await,
        Err(Error::InvalidInput)
    );
    assert_eq!(
        register.release_expired(due, 1001).await,
        Err(Error::InvalidInput)
    );
}

async fn invalid_operations<S: Storage + Clone>(storage: S) {
    let register = Register::new(storage.clone(), ReleasePeriod::default());
    let mut r = reservation(1, "invalid_input");
    r.expires_at = now();
    assert_eq!(
        register.reserve_handle(r.clone(), now()).await,
        Err(Error::InvalidInput)
    );
    r.expires_at += Duration::nanoseconds(1);
    assert_eq!(
        register.reserve_handle(r, now()).await,
        Err(Error::InvalidInput)
    );
    assert_eq!(
        register.set_role(&id(1), Role::Root).await,
        Err(Error::MemberNotFound)
    );
    assert_eq!(
        register.extend_lease(&id(1), month(2027, 1), now()).await,
        Err(Error::MemberNotFound)
    );
    assert_eq!(
        register
            .change_handle(&id(1), handle("missing"), now())
            .await,
        Err(Error::MemberNotFound)
    );
    let invalid_now = time("0000-01-01T00:00:00Z");
    assert_eq!(
        register
            .is_handle_available(handle("missing").skeleton(), invalid_now)
            .await,
        Err(Error::InvalidInput)
    );
    assert_eq!(
        register.is_handle_available("", now()).await,
        Err(Error::InvalidInput)
    );
    enroll(&register, 1, "lapsed_member", month(2026, 9)).await;
    let later = time("2026-11-01T00:00:00Z");
    assert_eq!(
        register
            .change_handle(&id(1), handle("new_name"), later)
            .await,
        Err(Error::LeaseExpired)
    );
    assert_eq!(
        register.extend_lease(&id(1), month(2026, 10), later).await,
        Err(Error::LeaseExpired)
    );
    assert_eq!(
        register.member(&id(1)).await.unwrap().unwrap().lease_end,
        month(2026, 9)
    );
    let overflowing = Register::new(storage.clone(), ReleasePeriod::months(u32::MAX));
    assert_eq!(
        overflowing
            .change_handle(&id(1), handle("overflow"), now())
            .await,
        Err(Error::DateOverflow)
    );
    assert!(
        register
            .is_handle_available(handle("overflow").skeleton(), now())
            .await
            .unwrap()
    );
    assert!(
        register
            .by_handle(handle("lapsed_member").skeleton(), now())
            .await
            .unwrap()
            .is_some()
    );
    register
        .reserve_handle(reservation(2, "late_lease"), now())
        .await
        .unwrap();
    assert_eq!(
        register
            .admit(
                Admission {
                    id: id(2),
                    handle: handle("late_lease"),
                    role: Role::Member,
                    lease_end: month(9999, 12)
                },
                now()
            )
            .await,
        Err(Error::DateOverflow)
    );
    assert!(register.member(&id(2)).await.unwrap().is_none());
    assert!(
        !register
            .is_handle_available(handle("late_lease").skeleton(), now())
            .await
            .unwrap()
    );
}

async fn rollback<S: Storage + Clone>(storage: S) {
    let mut tx = storage.begin().await.unwrap();
    tx.save_claim("first", &Claim::Reserved(reservation(1, "first")))
        .await
        .unwrap();
    drop(tx);
    let mut tx = storage.begin().await.unwrap();
    assert_eq!(tx.claim(handle("first").skeleton()).await.unwrap(), None);
    tx.save_claim("first", &Claim::Reserved(reservation(1, "first")))
        .await
        .unwrap();
    assert_eq!(
        tx.save_claim("second", &Claim::Reserved(reservation(1, "second")))
            .await,
        Err(Error::Storage)
    );
    assert_eq!(tx.commit().await, Err(Error::Storage));
    let mut tx = storage.begin().await.unwrap();
    assert_eq!(tx.claim(handle("first").skeleton()).await.unwrap(), None);
    assert_eq!(tx.claim(handle("second").skeleton()).await.unwrap(), None);
    tx.commit().await.unwrap();
}

async fn competing_claims<S: Storage + Clone + 'static>(storage: S) {
    let register = Register::new(storage, ReleasePeriod::default());
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(8));
    let mut tasks = Vec::new();
    for value in 1..=8 {
        let register = register.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            register
                .reserve_handle(reservation(value, "contested"), now())
                .await
        }));
    }
    let mut winners = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => winners += 1,
            Err(Error::HandleUnavailable) => {}
            other => panic!("unexpected result: {other:?}"),
        }
    }
    assert_eq!(winners, 1);
}

macro_rules! backend_tests {
    ($module:ident, $setup:ident, $($test:ident),+ $(,)?) => {
        mod $module {
            use super::*;
            $(#[tokio::test]
            async fn $test() {
                let (_directory, storage) = $setup().await;
                super::$test(storage).await;
            })+
        }
    };
}
backend_tests!(
    in_memory,
    memory,
    lifecycle,
    uniqueness,
    reservation_expiry,
    release_boundaries,
    lazy_release_and_renewal,
    handle_changes,
    policy_deadlines,
    bounded_cleanup,
    invalid_operations,
    rollback,
    competing_claims
);
backend_tests!(
    libsql,
    sql,
    lifecycle,
    uniqueness,
    reservation_expiry,
    release_boundaries,
    lazy_release_and_renewal,
    handle_changes,
    policy_deadlines,
    bounded_cleanup,
    invalid_operations,
    rollback,
    competing_claims
);

#[test]
fn dates_and_redaction() {
    for input in [
        "0000-01",
        "2026-00",
        "2026-13",
        "26-09",
        "2026-9",
        "２０２６-09",
        "2026-09-01",
        "2026-09 ",
    ] {
        assert!(input.parse::<YearMonth>().is_err(), "{input}");
    }
    assert_eq!(month(2026, 12).checked_add(2).unwrap(), month(2027, 2));
    assert_eq!(month(2024, 2).checked_add(12).unwrap(), month(2025, 2));
    assert_eq!(month(9999, 12).checked_add(1), Err(Error::DateOverflow));
    assert_eq!(
        month(2026, 9).checked_add(u32::MAX),
        Err(Error::DateOverflow)
    );
    assert_eq!(month(1, 1).to_string(), "0001-01");
    assert_eq!(month(2026, 9).year(), 2026);
    assert_eq!(month(2026, 9).month(), 9);
    assert_eq!(ReleasePeriod::default().as_months(), 24);
    assert!(MemberId::new(Vec::new()).is_err());
    assert!(Handle::new("", "skeleton").is_err());
    assert!(Handle::new("display", "").is_err());
    assert!(Handle::new("display\0", "skeleton").is_err());
    assert!(Handle::new("display", "skeleton\0").is_err());
    let private = MemberId::new(b"private_member_id".to_vec()).unwrap();
    let name = handle("private_display");
    let debug = format!("{private:?} {name:?} {:?}", Error::Storage);
    for secret in ["private_member_id", "private_display", "private_skeleton"] {
        assert!(!debug.contains(secret));
    }
}

#[test]
fn supplied_skeletons_must_match_normalized_displays() {
    let expected = handle("paypalname");
    for display in ["PAYPALNAME", "ｐａｙｐａｌｎａｍｅ", "pаypalname"] {
        assert_eq!(handle(display).skeleton(), expected.skeleton());
        assert!(Handle::new(display, expected.skeleton()).is_ok());
        assert_eq!(
            Handle::new(display, "invented-unique-key"),
            Err(Error::InvalidInput)
        );
    }
    for input in ["has space", "zero\0byte", "", &"a".repeat(4097)] {
        assert!(Handle::from_display(input).is_err());
    }
}
