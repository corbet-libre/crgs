//! Actual retained authority at the original register role transaction.
use super::*;
use std::future::Future;

/// Purpose-specific caller authority for an original role change.
/// The register supplies the actual BEFORE member from its held writer; the
/// caller rechecks original actor/target/key/legal/session owners without any
/// nested writer. It must retain original deadlines and same-second cut.
pub trait RoleCommitGuard: Send {
    /// Recheck actual owners after the update is staged but before commit.
    /// `current` is original stored data, not a caller-selected role assertion.
    fn check(
        &mut self,
        current: &Member,
        requested: Role,
    ) -> impl Future<Output = Result<()>> + Send;
    /// Final synchronous boundary before commit and after acknowledgment.
    /// A post-commit failure requires reconciliation, never a rollback claim.
    fn release(&mut self) -> Result<()>;
}

impl<S: Storage> Register<S> {
    /// Change only the original role under the same actual register writer,
    /// checking retained current authority at its logical commit boundary.
    /// Caller cancellation drops the uncommitted transaction. An error after
    /// acknowledgment is Storage and may leave the new role durably installed.
    pub async fn set_role_checked<G: RoleCommitGuard>(
        &self,
        id: &MemberId,
        role: Role,
        guard: &mut G,
    ) -> Result<Member> {
        let mut tx = self.storage.begin().await?;
        let mut stored = tx.member(id).await?.ok_or(Error::MemberNotFound)?;
        let before = stored.member.clone();
        stored.member.role = role;
        tx.save_member(&stored).await?;
        guard.check(&before, role).await?;
        guard.release()?;
        tx.commit().await?;
        guard.release().map_err(|_| Error::Storage)?;
        Ok(stored.member)
    }
}
