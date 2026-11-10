use super::*;
use crate::{Admission, Register, ReleasePeriod, SCHEMA};
use crlt::{Config, Db, Migration};

async fn database() -> (tempfile::TempDir, Db) {
    let directory = tempfile::tempdir().unwrap();
    let db = Db::open(Config::new(
        format!("file://{}", directory.path().join("plans.db").display()),
        "",
    ))
    .await
    .unwrap();
    assert_eq!(
        db.migrate(&[Migration::new(1, "crgs", SCHEMA)])
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.migrate(&[Migration::new(1, "crgs", SCHEMA)])
            .await
            .unwrap(),
        0
    );
    (directory, db)
}

fn sample() -> StoredMember {
    StoredMember {
        member: Member {
            id: MemberId::new(vec![0, 255, 1]).unwrap(),
            handle: Some(Handle::from_display("member_name").unwrap()),
            role: Role::Member,
            lease_end: YearMonth::new(2026, 9).unwrap(),
        },
        release_on: Some(YearMonth::new(2028, 10).unwrap()),
    }
}

#[tokio::test]
async fn every_adapter_statement_uses_the_intended_index() {
    let (_directory, db) = database().await;
    let scope = db.community("plans").unwrap();
    let stored = sample();
    let id = Value::Blob(stored.member.id.as_bytes().to_vec());
    let skeleton = Value::Text("member_name".into());
    let date = Value::Text("2028-10".into());
    let claim = Claim::Active(stored.member.id.clone());
    let cases = [
        (MEMBER, vec![id.clone()], "member_id=?"),
        (CLAIM, vec![skeleton.clone()], "skeleton=?"),
        (RESERVATION, vec![id], "crgs_reservation_owner"),
        (INSERT_MEMBER, member_values(&stored), ""),
        (UPDATE_MEMBER, member_values(&stored), "member_id=?"),
        (INSERT_CLAIM, claim_values("member_name", &claim), ""),
        (
            UPDATE_CLAIM,
            claim_values("member_name", &claim),
            "skeleton=?",
        ),
        (DELETE_CLAIM, vec![skeleton], "skeleton=?"),
        (
            DUE_RESERVATIONS,
            vec![Value::Integer(1_800_000_000), Value::Integer(100)],
            "crgs_reservation_expiry",
        ),
        (
            DUE_BLOCKS,
            vec![date.clone(), Value::Integer(100)],
            "crgs_block_expiry",
        ),
        (
            DUE_MEMBERS,
            vec![date, Value::Integer(100)],
            "crgs_member_release",
        ),
    ];
    for (sql, values, expected) in cases {
        let plan = scope.explain(sql, values).await.unwrap();
        plan.assert_indexed().unwrap();
        if !expected.is_empty() {
            assert!(
                plan.details()
                    .iter()
                    .any(|detail| detail.contains(expected)),
                "{sql}: {plan:?}"
            );
            assert!(
                plan.details()
                    .iter()
                    .any(|detail| detail.contains("community_id=?")),
                "{sql}: {plan:?}"
            );
        }
    }
}

#[tokio::test]
async fn stored_fields_are_coarse_and_blocks_have_no_owner() {
    let (_directory, db) = database().await;
    let scope = db.community("privacy").unwrap();
    let register = Register::new(LibsqlStorage::new(scope.clone()), ReleasePeriod::default());
    let stored = sample();
    let now = "2026-09-30T12:34:56Z".parse().unwrap();
    let expiry = "2026-09-30T13:34:56Z".parse().unwrap();
    let handle = stored.member.handle.clone().unwrap();
    register
        .reserve_handle(
            Reservation {
                member_id: stored.member.id.clone(),
                handle: handle.clone(),
                expires_at: expiry,
            },
            now,
        )
        .await
        .unwrap();
    register
        .admit(
            Admission {
                id: stored.member.id.clone(),
                handle,
                role: Role::Member,
                lease_end: stored.member.lease_end,
            },
            now,
        )
        .await
        .unwrap();
    register
        .change_handle(
            &stored.member.id,
            Handle::from_display("new_name").unwrap(),
            now,
        )
        .await
        .unwrap();
    let members = scope
        .query(
            "SELECT * FROM crgs_members WHERE member_id = ?1",
            params![stored.member.id.as_bytes().to_vec()],
        )
        .await
        .unwrap();
    assert_eq!(
        members[0].columns(),
        [
            "community_id",
            "member_id",
            "display_handle",
            "skeleton",
            "role",
            "lease_end",
            "release_on"
        ]
    );
    assert_eq!(members[0].get_str(5).unwrap(), "2026-09");
    assert_eq!(members[0].get_str(6).unwrap(), "2028-10");
    let claims = scope
        .query(
            CLAIM,
            [Handle::from_display("member_name").unwrap().skeleton()],
        )
        .await
        .unwrap();
    assert_eq!(claims[0].get_str(1).unwrap(), "blocked");
    for column in [2, 3, 4] {
        assert!(matches!(claims[0].get_value(column).unwrap(), Value::Null));
    }
    assert_eq!(claims[0].get_str(5).unwrap(), "2028-10");
    let active = scope
        .query(
            CLAIM,
            [Handle::from_display("new_name").unwrap().skeleton()],
        )
        .await
        .unwrap();
    assert!(matches!(active[0].get_value(4).unwrap(), Value::Null));
}

#[tokio::test]
async fn schema_rejects_invalid_roles_months_and_claim_shapes() {
    let (_directory, db) = database().await;
    let scope = db.community("constraints").unwrap();
    for lease in ["2026-13", "0000-01", "2026-09-30", "2026-00"] {
        let mut values = member_values(&sample());
        values[4] = Value::Text(lease.to_owned());
        assert!(scope.execute(INSERT_MEMBER, values).await.is_err());
    }
    let mut values = member_values(&sample());
    values[3] = Value::Text("operator".to_owned());
    assert!(scope.execute(INSERT_MEMBER, values).await.is_err());
    let mut values = claim_values(
        "old_name",
        &Claim::Blocked(YearMonth::new(2028, 10).unwrap()),
    );
    values[2] = Value::Blob(vec![1]);
    assert!(scope.execute(INSERT_CLAIM, values).await.is_err());
    assert!(
        scope
            .query(MEMBER, params![vec![0_u8, 255, 1]])
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_failed_decode_refuses_to_yield_an_owned_writer() {
    let (_directory, db) = database().await;
    let scope = db.community("failed_writer").unwrap();
    let stored = sample();
    let storage = LibsqlStorage::new(scope.clone());
    let mut tx = storage.begin().await.unwrap();
    tx.save_member(&stored).await.unwrap();
    tx.commit().await.unwrap();
    // A damaged import: the stored skeleton no longer matches its display name.
    scope
        .execute(
            "UPDATE crgs_members SET skeleton = ?1 WHERE member_id = ?2",
            params!["different", stored.member.id.as_bytes().to_vec()],
        )
        .await
        .unwrap();
    let mut tx = storage.begin().await.unwrap();
    assert_eq!(tx.member(&stored.member.id).await, Err(Error::Storage));
    // Fail closed: a writer that already saw a decode failure is never usable.
    assert_eq!(
        tx.into_owned_transaction().err(),
        Some(Error::Storage),
        "a failed wrapper must not become an owned writer"
    );
}

#[tokio::test]
async fn an_advisory_read_cannot_become_an_owned_write_transaction() {
    let (_directory, db) = database().await;
    let storage = LibsqlStorage::new(db.community("reads").unwrap());
    let tx = storage.begin_read().await.unwrap();
    assert_eq!(
        tx.into_owned_transaction().err(),
        Some(Error::Storage),
        "an advisory read capability must never be converted into a writer"
    );
}

#[tokio::test]
async fn missing_schema_is_a_redacted_storage_failure() {
    let directory = tempfile::tempdir().unwrap();
    let db = Db::open(Config::new(
        format!("file://{}", directory.path().join("missing.db").display()),
        "",
    ))
    .await
    .unwrap();
    let register = Register::new(
        LibsqlStorage::new(db.community("private_scope").unwrap()),
        ReleasePeriod::default(),
    );
    let error = register
        .member(&MemberId::new(b"private_member".to_vec()).unwrap())
        .await
        .unwrap_err();
    assert_eq!(error, Error::Storage);
    assert_eq!(error.to_string(), "register storage failure");
}
