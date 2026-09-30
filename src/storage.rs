//! Transactional storage primitives. Implementors must preserve atomicity and isolation.

use std::future::Future;

use crate::{Member, MemberId, Reservation, Result, YearMonth};

/// Persisted member plus its prospective, coarse release deadline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMember {
    /// Public member data.
    pub member: Member,
    /// First month in which its handle can be released; absent after release.
    pub release_on: Option<YearMonth>,
}

/// Exclusive claim on a skeleton; blocks deliberately contain no former owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Claim {
    /// Temporary pending enrolment claim.
    Reserved(Reservation),
    /// Current member's handle.
    Active(MemberId),
    /// Old skeleton, unavailable until the start of this month.
    Blocked(YearMonth),
}

/// A single, fixed community's transactional storage capability.
pub trait Storage: Send + Sync {
    /// An isolated transaction whose uncommitted changes disappear on drop.
    type Transaction: Transaction;

    /// Open an advisory read capability without acquiring a database write lock.
    /// Writes through it must fail. Results may change before a later mutation.
    fn begin_read(&self) -> impl Future<Output = Result<Self::Transaction>> + Send;

    /// Serialize writers before any reads. No dirty reads or lost updates are allowed.
    fn begin(&self) -> impl Future<Output = Result<Self::Transaction>> + Send;
}

/// Minimal storage operations used by the register's shared lifecycle rules.
///
/// The member and skeleton indexes must be unique within the fixed community.
/// A member may have at most one reserved claim and one active claim. Any
/// storage failure must prevent commit; dropping or cancelling rolls back.
pub trait Transaction: Send {
    /// Read one member by opaque ID.
    fn member(
        &mut self,
        id: &MemberId,
    ) -> impl Future<Output = Result<Option<StoredMember>>> + Send;
    /// Read one skeleton claim.
    fn claim(&mut self, skeleton: &str) -> impl Future<Output = Result<Option<Claim>>> + Send;
    /// Read the single pending reservation for a member.
    fn reservation(
        &mut self,
        id: &MemberId,
    ) -> impl Future<Output = Result<Option<Reservation>>> + Send;
    /// Insert or replace a member record within this transaction.
    fn save_member(&mut self, member: &StoredMember) -> impl Future<Output = Result<()>> + Send;
    /// Insert or replace a skeleton's claim within this transaction.
    fn save_claim(
        &mut self,
        skeleton: &str,
        claim: &Claim,
    ) -> impl Future<Output = Result<()>> + Send;
    /// Remove a claim by skeleton.
    fn remove_claim(&mut self, skeleton: &str) -> impl Future<Output = Result<()>> + Send;
    /// Fetch at most `limit` expired reservations, using an expiry index.
    fn due_reservations(
        &mut self,
        seconds: i64,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;
    /// Fetch at most `limit` expired blocks, using an expiry index.
    fn due_blocks(
        &mut self,
        month: YearMonth,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<String>>> + Send;
    /// Fetch at most `limit` members with releasable handles, using an expiry index.
    fn due_members(
        &mut self,
        month: YearMonth,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<MemberId>>> + Send;
    /// Atomically commit all changes. A failed commit can have an uncertain remote outcome.
    fn commit(self) -> impl Future<Output = Result<()>> + Send;
}
