use chrono::{DateTime, Utc};

mod admission;
mod first;
mod role;
pub use role::RoleCommitGuard;

use crate::{
    Admission, Error, Handle, Member, MemberId, ReleasePeriod, Released, Reservation, Result, Role,
    Storage, Transaction, YearMonth,
    storage::{Claim, StoredMember},
};

/// Community register with shared lifecycle rules and a caller-selected release policy.
#[derive(Clone)]
pub struct Register<S> {
    storage: S,
    period: ReleasePeriod,
}

impl<S: Storage> Register<S> {
    /// Bind the register to one community capability and a retention policy.
    pub fn new(storage: S, period: ReleasePeriod) -> Self {
        Self { storage, period }
    }

    /// Reserve one handle for a pending member. Exact retries do not extend expiry.
    pub async fn reserve_handle(
        &self,
        reservation: Reservation,
        now: DateTime<Utc>,
    ) -> Result<Reservation> {
        YearMonth::at(now)?;
        YearMonth::at(reservation.expires_at)?;
        if reservation.expires_at <= now || reservation.expires_at.timestamp_subsec_nanos() != 0 {
            return Err(Error::InvalidInput);
        }
        let mut tx = self.storage.begin().await?;
        if tx.member(&reservation.member_id).await?.is_some() {
            return Err(Error::MemberExists);
        }
        if let Some(previous) = tx.reservation(&reservation.member_id).await? {
            if previous.expires_at > now {
                if previous != reservation {
                    return Err(Error::ReservationConflict);
                }
                tx.commit().await?;
                return Ok(previous);
            }
            tx.remove_claim(previous.handle.skeleton()).await?;
        }
        let skeleton = reservation.handle.skeleton();
        if live_claim(&mut tx, skeleton, now, true).await?.is_some() {
            return Err(Error::HandleUnavailable);
        }
        tx.save_claim(skeleton, &Claim::Reserved(reservation.clone()))
            .await?;
        tx.commit().await?;
        Ok(reservation)
    }

    /// Cancel only the matching owner's pending claim. Repeating it is harmless.
    pub async fn cancel_reservation(&self, member: &MemberId, skeleton: &str) -> Result<bool> {
        let mut tx = self.storage.begin().await?;
        let matches = matches!(tx.claim(skeleton).await?, Some(Claim::Reserved(ref r)) if &r.member_id == member);
        if matches {
            tx.remove_claim(skeleton).await?;
        }
        tx.commit().await?;
        Ok(matches)
    }

    /// Admit an exact live reservation, or return an identical prior admission.
    pub async fn admit(&self, admission: Admission, now: DateTime<Utc>) -> Result<Member> {
        YearMonth::at(now)?;
        let mut tx = self.storage.begin().await?;
        let member = stage_admission(&mut tx, self.period, admission, now).await?;
        tx.commit().await?;
        Ok(member)
    }

    /// Read stored member state without recording access or running maintenance.
    /// The stored handle may be due for release; use `by_handle` for a live lookup.
    pub async fn member(&self, id: &MemberId) -> Result<Option<Member>> {
        let mut tx = self.storage.begin_read().await?;
        let member = tx.member(id).await?.map(|stored| stored.member);
        tx.commit().await?;
        Ok(member)
    }

    /// Read the current reservation without locking writers or performing cleanup.
    pub async fn reservation(&self, id: &MemberId) -> Result<Option<Reservation>> {
        let mut tx = self.storage.begin_read().await?;
        let reservation = tx.reservation(id).await?;
        tx.commit().await?;
        Ok(reservation)
    }

    /// Read the stored role, independently of handle or gate eligibility.
    pub async fn role(&self, id: &MemberId) -> Result<Option<Role>> {
        Ok(self.member(id).await?.map(|member| member.role))
    }

    /// Read a live active handle; expired claims are invisible even before cleanup.
    pub async fn by_handle(&self, skeleton: &str, now: DateTime<Utc>) -> Result<Option<Member>> {
        YearMonth::at(now)?;
        let mut tx = self.storage.begin_read().await?;
        let member = match live_claim(&mut tx, skeleton, now, false).await? {
            Some(Claim::Active(id)) => Some(tx.member(&id).await?.ok_or(Error::Storage)?.member),
            _ => None,
        };
        tx.commit().await?;
        Ok(member)
    }

    /// Check availability without changing state. The answer is advisory until a write commits.
    pub async fn is_handle_available(&self, skeleton: &str, now: DateTime<Utc>) -> Result<bool> {
        if skeleton.is_empty() || skeleton.contains('\0') {
            return Err(Error::InvalidInput);
        }
        YearMonth::at(now)?;
        let mut tx = self.storage.begin_read().await?;
        let available = live_claim(&mut tx, skeleton, now, false).await?.is_none();
        tx.commit().await?;
        Ok(available)
    }

    /// Store a role already authorized by the caller.
    pub async fn set_role(&self, id: &MemberId, role: Role) -> Result<Member> {
        let mut tx = self.storage.begin().await?;
        let mut stored = tx.member(id).await?.ok_or(Error::MemberNotFound)?;
        stored.member.role = role;
        tx.save_member(&stored).await?;
        tx.commit().await?;
        Ok(stored.member)
    }

    /// Extend monotonically; never resurrect a handle already due for release.
    /// Released members return `LeaseExpired`. A due lazy release returns the
    /// unchanged lease with no handle. Only unreleased members can renew.
    /// A shorter or identical live lease is a no-op; renewal is caller-authorized.
    pub async fn extend_lease(
        &self,
        id: &MemberId,
        lease_end: YearMonth,
        now: DateTime<Utc>,
    ) -> Result<Member> {
        let current = YearMonth::at(now)?;
        let mut tx = self.storage.begin().await?;
        let mut stored = tx.member(id).await?.ok_or(Error::MemberNotFound)?;
        if stored.member.handle.is_none() {
            return Err(Error::LeaseExpired);
        }
        if stored.release_on.is_some_and(|until| until <= current) {
            release_member(&mut tx, &mut stored).await?;
            tx.commit().await?;
            return Ok(stored.member);
        }
        if lease_end <= stored.member.lease_end {
            tx.commit().await?;
            return Ok(stored.member);
        }
        if lease_end < current {
            return Err(Error::LeaseExpired);
        }
        let new_release = self.period.after(lease_end)?;
        stored.member.lease_end = lease_end;
        if stored.member.handle.is_some() {
            stored.release_on = Some(
                stored
                    .release_on
                    .map_or(new_release, |old| old.max(new_release)),
            );
        }
        tx.save_member(&stored).await?;
        tx.commit().await?;
        Ok(stored.member)
    }

    /// Claim a new handle and block the former skeleton atomically.
    /// Requires a current lease. Exact retries and display-only changes create no block.
    pub async fn change_handle(
        &self,
        id: &MemberId,
        handle: Handle,
        now: DateTime<Utc>,
    ) -> Result<Member> {
        let current = YearMonth::at(now)?;
        let mut tx = self.storage.begin().await?;
        let mut stored = tx.member(id).await?.ok_or(Error::MemberNotFound)?;
        if stored.member.handle.is_none() || stored.member.lease_end < current {
            return Err(Error::LeaseExpired);
        }
        let previous = stored.member.handle.clone();
        if previous
            .as_ref()
            .is_some_and(|old| old.skeleton() == handle.skeleton())
        {
            stored.member.handle = Some(handle);
            tx.save_member(&stored).await?;
            tx.commit().await?;
            return Ok(stored.member);
        }
        if live_claim(&mut tx, handle.skeleton(), now, true)
            .await?
            .is_some()
        {
            return Err(Error::HandleUnavailable);
        }
        let block_until = self.period.after(current)?;
        let release_on = self.period.after(stored.member.lease_end)?;
        if let Some(old) = previous {
            tx.save_claim(old.skeleton(), &Claim::Blocked(block_until))
                .await?;
        }
        tx.save_claim(handle.skeleton(), &Claim::Active(id.clone()))
            .await?;
        stored.member.handle = Some(handle);
        stored.release_on = Some(
            stored
                .release_on
                .map_or(release_on, |old| old.max(release_on)),
        );
        tx.save_member(&stored).await?;
        tx.commit().await?;
        Ok(stored.member)
    }

    /// Release up to `limit` rows of each expiry kind (1–1000) in one transaction.
    pub async fn release_expired(&self, now: DateTime<Utc>, limit: usize) -> Result<Released> {
        if !(1..=1000).contains(&limit) {
            return Err(Error::InvalidInput);
        }
        let month = YearMonth::at(now)?;
        let mut tx = self.storage.begin().await?;
        let reservations = tx.due_reservations(now.timestamp(), limit).await?;
        let blocks = tx.due_blocks(month, limit).await?;
        let members = tx.due_members(month, limit).await?;
        for skeleton in reservations.iter().chain(&blocks) {
            tx.remove_claim(skeleton).await?;
        }
        for id in &members {
            let mut stored = tx.member(id).await?.ok_or(Error::Storage)?;
            release_member(&mut tx, &mut stored).await?;
        }
        tx.commit().await?;
        Ok(Released {
            reservations: reservations.len(),
            handles: members.len(),
            blocks: blocks.len(),
        })
    }
}

async fn release_member(tx: &mut impl Transaction, stored: &mut StoredMember) -> Result<()> {
    let handle = stored.member.handle.take().ok_or(Error::Storage)?;
    match tx.claim(handle.skeleton()).await? {
        Some(Claim::Active(id)) if id == stored.member.id => {}
        _ => return Err(Error::Storage),
    }
    tx.remove_claim(handle.skeleton()).await?;
    stored.release_on = None;
    tx.save_member(stored).await
}

async fn live_claim(
    tx: &mut impl Transaction,
    skeleton: &str,
    now: DateTime<Utc>,
    cleanup: bool,
) -> Result<Option<Claim>> {
    let claim = tx.claim(skeleton).await?;
    let month = YearMonth::at(now)?;
    let expired = match &claim {
        None => return Ok(None),
        Some(Claim::Reserved(r)) => r.expires_at <= now,
        Some(Claim::Blocked(until)) => *until <= month,
        Some(Claim::Active(id)) => {
            let mut stored = tx.member(id).await?.ok_or(Error::Storage)?;
            if stored.member.handle.as_ref().map(Handle::skeleton) != Some(skeleton) {
                return Err(Error::Storage);
            }
            let due = stored.release_on.ok_or(Error::Storage)? <= month;
            if due && cleanup {
                release_member(tx, &mut stored).await?;
            }
            due
        }
    };
    if expired {
        if cleanup && !matches!(claim, Some(Claim::Active(_))) {
            tx.remove_claim(skeleton).await?;
        }
        Ok(None)
    } else {
        Ok(claim)
    }
}

/// Shared original admission lifecycle; the caller retains commit ownership.
async fn stage_admission(
    tx: &mut impl Transaction,
    period: ReleasePeriod,
    admission: Admission,
    now: DateTime<Utc>,
) -> Result<Member> {
    let current = YearMonth::at(now)?;
    let member = Member {
        id: admission.id,
        handle: Some(admission.handle),
        role: admission.role,
        lease_end: admission.lease_end,
    };
    if let Some(existing) = tx.member(&member.id).await? {
        if existing.member != member {
            return Err(Error::MemberExists);
        }
        return Ok(existing.member);
    }
    if member.lease_end < current {
        return Err(Error::LeaseExpired);
    }
    let release_on = period.after(member.lease_end)?;
    let handle = member.handle.as_ref().ok_or(Error::InvalidInput)?;
    match tx.claim(handle.skeleton()).await? {
        Some(Claim::Reserved(r))
            if r.member_id == member.id && r.handle == *handle && r.expires_at > now => {}
        _ => return Err(Error::ReservationRequired),
    }
    tx.save_member(&StoredMember {
        member: member.clone(),
        release_on: Some(release_on),
    })
    .await?;
    tx.save_claim(handle.skeleton(), &Claim::Active(member.id.clone()))
        .await?;
    Ok(member)
}
