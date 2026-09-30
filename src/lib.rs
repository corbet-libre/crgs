#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod memory;
mod register;
mod sql;
pub mod storage;
mod types;

pub use memory::MemoryStorage;
pub use register::Register;
pub use sql::{LibsqlStorage, SCHEMA};
pub use storage::{Storage, Transaction};
pub use types::{
    Admission, Handle, Member, MemberId, ReleasePeriod, Released, Reservation, Role, YearMonth,
};

/// Register operation result. Errors never embed caller data or driver diagnostics.
pub type Result<T> = std::result::Result<T, Error>;

/// Fixed, data-free failure categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Invalid structural input or a deadline outside the supported calendar.
    #[error("invalid register input")]
    InvalidInput,
    /// Retention arithmetic exceeds the supported calendar.
    #[error("calendar arithmetic overflow")]
    DateOverflow,
    /// A different live claim holds the skeleton.
    #[error("handle unavailable")]
    HandleUnavailable,
    /// This member already has different admission data.
    #[error("member already exists")]
    MemberExists,
    /// The member does not exist.
    #[error("member not found")]
    MemberNotFound,
    /// Admission lacks the exact live reservation.
    #[error("matching live reservation required")]
    ReservationRequired,
    /// The member already holds a different pending reservation.
    #[error("conflicting reservation")]
    ReservationConflict,
    /// A released or expired lease cannot support a new handle without renewal.
    #[error("current lease required")]
    LeaseExpired,
    /// Storage failure; remote write outcome may require reconciliation.
    #[error("register storage failure")]
    Storage,
}

impl From<crlt::Error> for Error {
    fn from(_: crlt::Error) -> Self {
        Self::Storage
    }
}
