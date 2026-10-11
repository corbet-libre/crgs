//! The original renewal lifecycle inside the caller's same scoped writer.
use super::*;

impl Register<crate::LibsqlStorage> {
    /// Stage original monotonic renewal/lazy release without starting or
    /// committing a writer. A same-handle database/scope is mandatory; errors
    /// or cancellation drop every staged participating owner change together.
    /// This port does not prove admission, funding, role or policy authority.
    pub async fn extend_lease_in_transaction(
        &self,
        tx: crlt::Transaction,
        id: &MemberId,
        lease_end: YearMonth,
        now: DateTime<Utc>,
    ) -> Result<(crlt::Transaction, Member)> {
        let mut tx = self.storage.owned_transaction(tx)?;
        let member = stage_renewal(&mut tx, self.period, id, lease_end, now).await?;
        Ok((tx.into_owned_transaction()?, member))
    }
}

pub(super) async fn stage_renewal(
    tx: &mut impl Transaction,
    period: ReleasePeriod,
    id: &MemberId,
    lease_end: YearMonth,
    now: DateTime<Utc>,
) -> Result<Member> {
    let current = YearMonth::at(now)?;
    let mut stored = tx.member(id).await?.ok_or(Error::MemberNotFound)?;
    if stored.member.handle.is_none() {
        return Err(Error::LeaseExpired);
    }
    if stored.release_on.is_some_and(|until| until <= current) {
        release_member(tx, &mut stored).await?;
        return Ok(stored.member);
    }
    if lease_end <= stored.member.lease_end {
        return Ok(stored.member);
    }
    if lease_end < current {
        return Err(Error::LeaseExpired);
    }
    let new_release = period.after(lease_end)?;
    stored.member.lease_end = lease_end;
    if stored.member.handle.is_some() {
        stored.release_on = Some(
            stored
                .release_on
                .map_or(new_release, |old| old.max(new_release)),
        );
    }
    tx.save_member(&stored).await?;
    Ok(stored.member)
}
