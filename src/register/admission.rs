//! Original admission staging inside a caller-owned multi-owner transaction.
use super::*;

impl Register<crate::LibsqlStorage> {
    /// Stage the exact original reservation/admission lifecycle without
    /// beginning or committing a transaction. The writer must belong to this
    /// exact database handle and community. Return it only on success so an
    /// error or cancellation drops every staged owner change together.
    ///
    /// This is a storage composition port, not admission or role authority.
    /// The caller retains all genuine policy/key/legal/voucher checks and owns
    /// the sole final commit after every participating owner has succeeded.
    pub async fn admit_in_transaction(
        &self,
        tx: crlt::Transaction,
        admission: Admission,
        now: DateTime<Utc>,
    ) -> Result<(crlt::Transaction, Member)> {
        let mut tx = self.storage.owned_transaction(tx)?;
        let member = stage_admission(&mut tx, self.period, admission, now).await?;
        Ok((tx.into_owned_transaction()?, member))
    }
}
