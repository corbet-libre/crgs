//! Original decoded member state in the caller's retained native writer.
use super::*;

impl Register<crate::LibsqlStorage> {
    /// Read the exact stored member without admission, renewal or maintenance.
    /// The caller's writer must belong to this exact database and community.
    /// No role or current eligibility is inferred from a present row.
    /// Return ownership only on success; refusal/cancellation drops every prior
    /// staged participating owner change. The caller alone owns final commit.
    pub async fn member_in_transaction(
        &self,
        tx: crlt::Transaction,
        id: &MemberId,
    ) -> Result<(crlt::Transaction, Option<Member>)> {
        let mut tx = self.storage.owned_transaction(tx)?;
        let member = tx.member(id).await?.map(|stored| stored.member);
        Ok((tx.into_owned_transaction()?, member))
    }
}
