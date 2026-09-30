use std::{collections::BTreeMap, sync::Arc};

use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::{
    Error, MemberId, Reservation, Result, Storage, Transaction, YearMonth,
    storage::{Claim, StoredMember},
};

/// Real in-memory storage for one community. Clones share committed state.
#[derive(Clone, Default)]
pub struct MemoryStorage(Arc<Mutex<State>>);

#[derive(Clone, Default)]
struct State {
    members: BTreeMap<MemberId, StoredMember>,
    claims: BTreeMap<String, Claim>,
}

/// Isolated in-memory transaction; dropping it discards its working copy.
pub struct MemoryTransaction {
    guard: Option<OwnedMutexGuard<State>>,
    working: State,
    failed: bool,
}

impl Storage for MemoryStorage {
    type Transaction = MemoryTransaction;

    async fn begin_read(&self) -> Result<Self::Transaction> {
        let working = self.0.lock().await.clone();
        Ok(MemoryTransaction {
            guard: None,
            working,
            failed: false,
        })
    }

    async fn begin(&self) -> Result<Self::Transaction> {
        let guard = self.0.clone().lock_owned().await;
        let working = guard.clone();
        Ok(MemoryTransaction {
            guard: Some(guard),
            working,
            failed: false,
        })
    }
}

impl Transaction for MemoryTransaction {
    async fn member(&mut self, id: &MemberId) -> Result<Option<StoredMember>> {
        Ok(self.working.members.get(id).cloned())
    }

    async fn claim(&mut self, skeleton: &str) -> Result<Option<Claim>> {
        Ok(self.working.claims.get(skeleton).cloned())
    }

    async fn reservation(&mut self, id: &MemberId) -> Result<Option<Reservation>> {
        Ok(self.working.claims.values().find_map(|claim| match claim {
            Claim::Reserved(reservation) if &reservation.member_id == id => {
                Some(reservation.clone())
            }
            _ => None,
        }))
    }

    async fn save_member(&mut self, member: &StoredMember) -> Result<()> {
        if self.guard.is_none() {
            return Err(Error::Storage);
        }
        self.working
            .members
            .insert(member.member.id.clone(), member.clone());
        Ok(())
    }

    async fn save_claim(&mut self, skeleton: &str, claim: &Claim) -> Result<()> {
        if self.guard.is_none() {
            return Err(Error::Storage);
        }
        let duplicate = self.working.claims.iter().any(|(key, other)| {
            key != skeleton
                && match (claim, other) {
                    (Claim::Reserved(a), Claim::Reserved(b)) => a.member_id == b.member_id,
                    (Claim::Active(a), Claim::Active(b)) => a == b,
                    _ => false,
                }
        });
        if duplicate {
            self.failed = true;
            return Err(Error::Storage);
        }
        self.working
            .claims
            .insert(skeleton.to_owned(), claim.clone());
        Ok(())
    }

    async fn remove_claim(&mut self, skeleton: &str) -> Result<()> {
        if self.guard.is_none() {
            return Err(Error::Storage);
        }
        self.working.claims.remove(skeleton);
        Ok(())
    }

    async fn due_reservations(&mut self, seconds: i64, limit: usize) -> Result<Vec<String>> {
        Ok(self
            .working
            .claims
            .iter()
            .filter_map(|(key, claim)| match claim {
                Claim::Reserved(r) if r.expires_at.timestamp() <= seconds => Some(key.clone()),
                _ => None,
            })
            .take(limit)
            .collect())
    }

    async fn due_blocks(&mut self, month: YearMonth, limit: usize) -> Result<Vec<String>> {
        Ok(self
            .working
            .claims
            .iter()
            .filter_map(|(key, claim)| match claim {
                Claim::Blocked(until) if *until <= month => Some(key.clone()),
                _ => None,
            })
            .take(limit)
            .collect())
    }

    async fn due_members(&mut self, month: YearMonth, limit: usize) -> Result<Vec<MemberId>> {
        Ok(self
            .working
            .members
            .iter()
            .filter_map(|(id, stored)| {
                stored
                    .release_on
                    .filter(|until| *until <= month)
                    .map(|_| id.clone())
            })
            .take(limit)
            .collect())
    }

    async fn commit(mut self) -> Result<()> {
        if self.failed {
            return Err(Error::Storage);
        }
        if let Some(ref mut guard) = self.guard {
            **guard = self.working;
        }
        Ok(())
    }
}
