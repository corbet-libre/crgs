use chrono::DateTime;
use crlt::{Row, Value, params};

use crate::{
    Error, Handle, Member, MemberId, Reservation, Result, Role, Storage, Transaction, YearMonth,
    storage::{Claim, StoredMember},
};

/// Trusted DDL to include in the service's complete `crlt` migration history.
/// Apply once through `Db::migrate` before constructing the storage capability.
pub const SCHEMA: &str = include_str!("schema.sql");

const MEMBER: &str = "SELECT member_id, display_handle, skeleton, role, lease_end, release_on FROM crgs_members WHERE member_id = ?1";
const CLAIM: &str = "SELECT skeleton, kind, owner, display_handle, expires_at, blocked_until FROM crgs_claims WHERE skeleton = ?1";
const RESERVATION: &str = "SELECT skeleton, kind, owner, display_handle, expires_at, blocked_until FROM crgs_claims WHERE kind = 'reserved' AND owner = ?1";
const INSERT_MEMBER: &str = "INSERT INTO crgs_members (member_id, display_handle, skeleton, role, lease_end, release_on) VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
const UPDATE_MEMBER: &str = "UPDATE crgs_members SET display_handle = ?2, skeleton = ?3, role = ?4, lease_end = ?5, release_on = ?6 WHERE member_id = ?1";
const INSERT_CLAIM: &str = "INSERT INTO crgs_claims (skeleton, kind, owner, display_handle, expires_at, blocked_until) VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
const UPDATE_CLAIM: &str = "UPDATE crgs_claims SET kind = ?2, owner = ?3, display_handle = ?4, expires_at = ?5, blocked_until = ?6 WHERE skeleton = ?1";
const DELETE_CLAIM: &str = "DELETE FROM crgs_claims WHERE skeleton = ?1";
const DUE_RESERVATIONS: &str = "SELECT skeleton FROM crgs_claims WHERE kind = 'reserved' AND expires_at <= ?1 ORDER BY expires_at, skeleton LIMIT ?2";
const DUE_BLOCKS: &str = "SELECT skeleton FROM crgs_claims WHERE kind = 'blocked' AND blocked_until <= ?1 ORDER BY blocked_until, skeleton LIMIT ?2";
const DUE_MEMBERS: &str = "SELECT member_id FROM crgs_members WHERE release_on <= ?1 ORDER BY release_on, member_id LIMIT ?2";

/// Official libSQL storage through `crlt`, bound to a caller-selected community.
#[derive(Clone)]
pub struct LibsqlStorage {
    community: crlt::Community,
}

impl LibsqlStorage {
    /// Use a migrated community capability; connection settings remain with the service.
    pub fn new(community: crlt::Community) -> Self {
        Self { community }
    }

    pub(crate) fn owned_transaction(&self, tx: crlt::Transaction) -> Result<LibsqlTransaction> {
        if !tx.belongs_to(&self.community) {
            return Err(Error::Storage);
        }
        Ok(LibsqlTransaction {
            inner: Connection::Write(Box::new(tx)),
            failed: false,
        })
    }
}

/// Immediate SQL transaction. crlt discards uncommitted or failed connections.
pub struct LibsqlTransaction {
    inner: Connection,
    failed: bool,
}

impl Storage for LibsqlStorage {
    type Transaction = LibsqlTransaction;

    async fn begin_read(&self) -> Result<Self::Transaction> {
        Ok(LibsqlTransaction {
            inner: Connection::Read(self.community.clone()),
            failed: false,
        })
    }

    async fn begin(&self) -> Result<Self::Transaction> {
        Ok(LibsqlTransaction {
            inner: Connection::Write(Box::new(self.community.tx().await?)),
            failed: false,
        })
    }
}

enum Connection {
    // Boxed: crlt's transaction capability is several times the read variant and
    // clippy::large_enum_variant is denied in CI (`-D warnings`).
    Write(Box<crlt::Transaction>),
    Read(crlt::Community),
}
impl Connection {
    async fn query(&mut self, sql: &str, params: impl crlt::IntoParams) -> Result<Vec<Row>> {
        Ok(match self {
            Self::Write(tx) => tx.query(sql, params).await?,
            Self::Read(scope) => scope.query(sql, params).await?,
        })
    }
    async fn execute(&mut self, sql: &str, params: impl crlt::IntoParams) -> Result<u64> {
        match self {
            Self::Write(tx) => Ok(tx.execute(sql, params).await?),
            Self::Read(_) => Err(Error::Storage),
        }
    }
    async fn commit(self) -> Result<()> {
        if let Self::Write(tx) = self {
            tx.commit().await?;
        }
        Ok(())
    }
}

impl LibsqlTransaction {
    pub(crate) fn into_owned_transaction(self) -> Result<crlt::Transaction> {
        if self.failed {
            return Err(Error::Storage);
        }
        match self.inner {
            Connection::Write(tx) => Ok(*tx),
            Connection::Read(_) => Err(Error::Storage),
        }
    }

    fn checked<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.failed = true;
        }
        result
    }
}

impl Transaction for LibsqlTransaction {
    async fn member(&mut self, id: &MemberId) -> Result<Option<StoredMember>> {
        let rows = self
            .inner
            .query(MEMBER, params![id.as_bytes().to_vec()])
            .await?;
        self.checked(rows.first().map(decode_member).transpose())
    }

    async fn claim(&mut self, skeleton: &str) -> Result<Option<Claim>> {
        let rows = self.inner.query(CLAIM, [skeleton]).await?;
        self.checked(rows.first().map(decode_claim).transpose())
    }

    async fn reservation(&mut self, id: &MemberId) -> Result<Option<Reservation>> {
        let rows = self
            .inner
            .query(RESERVATION, params![id.as_bytes().to_vec()])
            .await?;
        let result = rows
            .first()
            .map(|row| match decode_claim(row)? {
                Claim::Reserved(reservation) => Ok(reservation),
                _ => Err(Error::Storage),
            })
            .transpose();
        self.checked(result)
    }

    async fn save_member(&mut self, stored: &StoredMember) -> Result<()> {
        let sql = if self.member(&stored.member.id).await?.is_some() {
            UPDATE_MEMBER
        } else {
            INSERT_MEMBER
        };
        self.inner.execute(sql, member_values(stored)).await?;
        Ok(())
    }

    async fn save_claim(&mut self, skeleton: &str, claim: &Claim) -> Result<()> {
        let sql = if self.claim(skeleton).await?.is_some() {
            UPDATE_CLAIM
        } else {
            INSERT_CLAIM
        };
        self.inner
            .execute(sql, claim_values(skeleton, claim))
            .await?;
        Ok(())
    }

    async fn remove_claim(&mut self, skeleton: &str) -> Result<()> {
        self.inner.execute(DELETE_CLAIM, [skeleton]).await?;
        Ok(())
    }

    async fn due_reservations(&mut self, seconds: i64, limit: usize) -> Result<Vec<String>> {
        let rows = self
            .inner
            .query(DUE_RESERVATIONS, params![seconds, limit as i64])
            .await?;
        self.checked(strings(rows))
    }

    async fn due_blocks(&mut self, month: YearMonth, limit: usize) -> Result<Vec<String>> {
        let rows = self
            .inner
            .query(DUE_BLOCKS, params![month.to_string(), limit as i64])
            .await?;
        self.checked(strings(rows))
    }

    async fn due_members(&mut self, month: YearMonth, limit: usize) -> Result<Vec<MemberId>> {
        let rows = self
            .inner
            .query(DUE_MEMBERS, params![month.to_string(), limit as i64])
            .await?;
        self.checked(rows.iter().map(|row| decode_id(row, 0)).collect())
    }

    async fn commit(self) -> Result<()> {
        if self.failed {
            return Err(Error::Storage);
        }
        self.inner.commit().await?;
        Ok(())
    }
}

fn strings(rows: Vec<Row>) -> Result<Vec<String>> {
    rows.iter()
        .map(|row| Ok(row.get_str(0)?.to_owned()))
        .collect()
}

fn optional_text(value: Option<String>) -> Value {
    value.map(Value::Text).unwrap_or(Value::Null)
}

fn member_values(stored: &StoredMember) -> Vec<Value> {
    let member = &stored.member;
    vec![
        Value::Blob(member.id.as_bytes().to_vec()),
        optional_text(member.handle.as_ref().map(|h| h.display().to_owned())),
        optional_text(member.handle.as_ref().map(|h| h.skeleton().to_owned())),
        Value::Text(
            match member.role {
                Role::Root => "root",
                Role::Admin => "admin",
                Role::Member => "member",
            }
            .to_owned(),
        ),
        Value::Text(member.lease_end.to_string()),
        optional_text(stored.release_on.map(|date| date.to_string())),
    ]
}

fn claim_values(skeleton: &str, claim: &Claim) -> Vec<Value> {
    let (kind, owner, display, expires, blocked) = match claim {
        Claim::Reserved(r) => (
            "reserved",
            Value::Blob(r.member_id.as_bytes().to_vec()),
            Value::Text(r.handle.display().to_owned()),
            Value::Integer(r.expires_at.timestamp()),
            Value::Null,
        ),
        Claim::Active(id) => (
            "active",
            Value::Blob(id.as_bytes().to_vec()),
            Value::Null,
            Value::Null,
            Value::Null,
        ),
        Claim::Blocked(until) => (
            "blocked",
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Text(until.to_string()),
        ),
    };
    vec![
        Value::Text(skeleton.to_owned()),
        Value::Text(kind.to_owned()),
        owner,
        display,
        expires,
        blocked,
    ]
}

fn decode_id(row: &Row, index: usize) -> Result<MemberId> {
    match row.get_value(index)? {
        Value::Blob(bytes) => MemberId::new(bytes.clone()).map_err(|_| Error::Storage),
        _ => Err(Error::Storage),
    }
}

fn decode_member(row: &Row) -> Result<StoredMember> {
    let handle = match (row.get_value(1)?, row.get_value(2)?) {
        (Value::Null, Value::Null) => None,
        (Value::Text(display), Value::Text(skeleton)) => {
            Some(Handle::new(display, skeleton).map_err(|_| Error::Storage)?)
        }
        _ => return Err(Error::Storage),
    };
    let release_on = match row.get_value(5)? {
        Value::Null => None,
        Value::Text(value) => Some(value.parse().map_err(|_| Error::Storage)?),
        _ => return Err(Error::Storage),
    };
    let role = match row.get_str(3)? {
        "root" => Role::Root,
        "admin" => Role::Admin,
        "member" => Role::Member,
        _ => return Err(Error::Storage),
    };
    Ok(StoredMember {
        member: Member {
            id: decode_id(row, 0)?,
            handle,
            role,
            lease_end: row.get_str(4)?.parse().map_err(|_| Error::Storage)?,
        },
        release_on,
    })
}

fn decode_claim(row: &Row) -> Result<Claim> {
    match row.get_str(1)? {
        "reserved" => Ok(Claim::Reserved(Reservation {
            member_id: decode_id(row, 2)?,
            handle: Handle::new(row.get_str(3)?, row.get_str(0)?).map_err(|_| Error::Storage)?,
            expires_at: DateTime::from_timestamp(row.get_i64(4)?, 0).ok_or(Error::Storage)?,
        })),
        "active" => Ok(Claim::Active(decode_id(row, 2)?)),
        "blocked" => Ok(Claim::Blocked(
            row.get_str(5)?.parse().map_err(|_| Error::Storage)?,
        )),
        _ => Err(Error::Storage),
    }
}

#[cfg(test)]
#[path = "tests/sql.rs"]
mod tests;
