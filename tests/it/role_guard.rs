//! Persistence boundaries only; these probes do not assert member authority.
use chrono::{DateTime, Duration, Utc};
use crgs::{
    Admission, Error, Handle, LibsqlStorage, Member, MemberId, Register, ReleasePeriod,
    Reservation, Role, RoleCommitGuard, SCHEMA, YearMonth,
};

async fn fixture() -> (tempfile::TempDir, Register<LibsqlStorage>, Member) {
    let directory = tempfile::tempdir().unwrap();
    let db = crlt::Db::open(crlt::Config::new(
        format!("file://{}", directory.path().join("roles.db").display()),
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
    let now: DateTime<Utc> = "2026-09-30T12:00:00Z".parse().unwrap();
    let id = MemberId::new(vec![1, 2, 3]).unwrap();
    let handle = Handle::from_display("original_member").unwrap();
    register
        .reserve_handle(
            Reservation {
                member_id: id.clone(),
                handle: handle.clone(),
                expires_at: now + Duration::hours(1),
            },
            now,
        )
        .await
        .unwrap();
    let member = register
        .admit(
            Admission {
                id,
                handle,
                role: Role::Member,
                lease_end: YearMonth::new(2026, 12).unwrap(),
            },
            now,
        )
        .await
        .unwrap();
    (directory, register, member)
}

struct Cut {
    before: Member,
    refuse_check: bool,
    refuse_release: usize,
    releases: usize,
}
impl RoleCommitGuard for Cut {
    async fn check(&mut self, current: &Member, requested: Role) -> crgs::Result<()> {
        assert_eq!(current, &self.before);
        assert_eq!(requested, Role::Admin);
        if self.refuse_check {
            Err(Error::Storage)
        } else {
            Ok(())
        }
    }
    fn release(&mut self) -> crgs::Result<()> {
        self.releases += 1;
        if self.releases == self.refuse_release {
            Err(Error::InvalidInput)
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn original_sql_role_update_rolls_back_when_final_check_or_precommit_release_refuses() {
    let (_directory, register, before) = fixture().await;
    for (refuse_check, refuse_release) in [(true, 0), (false, 1)] {
        let mut cut = Cut {
            before: before.clone(),
            refuse_check,
            refuse_release,
            releases: 0,
        };
        assert!(
            register
                .set_role_checked(&before.id, Role::Admin, &mut cut)
                .await
                .is_err()
        );
        assert_eq!(
            register.member(&before.id).await.unwrap(),
            Some(before.clone())
        );
    }
}

#[tokio::test]
async fn postacknowledgment_refusal_keeps_actual_role_and_reports_storage_uncertainty() {
    let (_directory, register, before) = fixture().await;
    let mut cut = Cut {
        before: before.clone(),
        refuse_check: false,
        refuse_release: 2,
        releases: 0,
    };
    assert_eq!(
        register
            .set_role_checked(&before.id, Role::Admin, &mut cut)
            .await,
        Err(Error::Storage)
    );
    assert_eq!(cut.releases, 2);
    let mut expected = before;
    expected.role = Role::Admin;
    assert_eq!(register.member(&expected.id).await.unwrap(), Some(expected));
}

#[tokio::test]
async fn acknowledged_role_change_commits_the_new_role_after_both_releases() {
    let (_directory, register, before) = fixture().await;
    let mut cut = Cut {
        before: before.clone(),
        refuse_check: false,
        refuse_release: 0,
        releases: 0,
    };
    let changed = register
        .set_role_checked(&before.id, Role::Admin, &mut cut)
        .await
        .unwrap();
    assert_eq!(cut.releases, 2);
    assert_eq!(changed.role, Role::Admin);
    assert_eq!(changed.id, before.id);
    assert_eq!(changed.handle, before.handle);
    assert_eq!(changed.lease_end, before.lease_end);
    assert_eq!(register.member(&before.id).await.unwrap(), Some(changed));
}

struct Suspended {
    entered: Option<tokio::sync::oneshot::Sender<()>>,
}
impl RoleCommitGuard for Suspended {
    async fn check(&mut self, _: &Member, _: Role) -> crgs::Result<()> {
        // Signal only after the original SQL role update is staged.
        let _ = self.entered.take().unwrap().send(());
        std::future::pending().await
    }
    fn release(&mut self) -> crgs::Result<()> {
        panic!("cancelled check cannot release")
    }
}

#[tokio::test]
async fn cancellation_during_actual_staged_role_check_drops_the_uncommitted_writer() {
    let (_directory, register, before) = fixture().await;
    let (entered, received) = tokio::sync::oneshot::channel();
    let mut cut = Suspended {
        entered: Some(entered),
    };
    let mut writing = Box::pin(register.set_role_checked(&before.id, Role::Admin, &mut cut));
    tokio::select! {
        result = &mut writing => panic!("unexpected completion: {result:?}"),
        result = received => result.unwrap(),
    }
    drop(writing);
    assert_eq!(register.member(&before.id).await.unwrap(), Some(before));
}
