//! Conservative creation initialization against the original scoped Register.
use super::*;

impl Register<crate::LibsqlStorage> {
    /// Return this exact caller-owned writer only while the original Register
    /// has no member rows, including expired or released members. This bounded
    /// indexed probe supports first-creation initialization; it neither admits
    /// a member nor grants a role. Existing admission/retry semantics are intact.
    /// A foreign writer, failure or cancellation drops all staged owner changes.
    pub async fn ensure_empty_in_transaction(
        &self,
        tx: crlt::Transaction,
    ) -> Result<crlt::Transaction> {
        let mut tx = self
            .storage
            .owned_transaction(tx)?
            .into_owned_transaction()?;
        if !tx
            .query(
                "SELECT member_id FROM crgs_members LIMIT 1",
                crlt::params![],
            )
            .await?
            .is_empty()
        {
            return Err(Error::MemberExists);
        }
        Ok(tx)
    }
}
