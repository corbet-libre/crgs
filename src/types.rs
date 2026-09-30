use std::{fmt, str::FromStr};

use chrono::{DateTime, Datelike, Utc};
use precis_profiles::{UsernameCaseMapped, precis_core::profile::PrecisFastInvocation};

use crate::{Error, Result};

/// An opaque, community-specific pseudonym. Debug output is redacted.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MemberId(Vec<u8>);

impl MemberId {
    /// Accept nonempty opaque bytes without interpreting or deriving them.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(Error::InvalidInput);
        }
        Ok(Self(bytes))
    }

    /// Borrow the community pseudonym for storage or protocol use.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for MemberId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MemberId([redacted])")
    }
}

/// A display handle bound to its PRECIS-normalized UTS #39 uniqueness key.
#[derive(Clone, PartialEq, Eq)]
pub struct Handle {
    display: String,
    skeleton: String,
}

impl Handle {
    /// Verify a supplied skeleton against the display, never trusting a caller's key.
    /// Reserved-name, length and profanity policy still belongs to cgrd.
    pub fn new(display: impl Into<String>, skeleton: impl Into<String>) -> Result<Self> {
        let handle = Self::from_display(display)?;
        if handle.skeleton != skeleton.into() {
            return Err(Error::InvalidInput);
        }
        Ok(handle)
    }

    /// Derive the uniqueness key using the same pinned Unicode profiles as cgrd.
    /// Preserve the display spelling; reject invalid or oversized PRECIS input.
    pub fn from_display(display: impl Into<String>) -> Result<Self> {
        let display = display.into();
        if display.is_empty() || display.len() > 4096 {
            return Err(Error::InvalidInput);
        }
        let normalized =
            UsernameCaseMapped::enforce(display.as_str()).map_err(|_| Error::InvalidInput)?;
        let skeleton = unicode_security::skeleton(&normalized).collect();
        Ok(Self { display, skeleton })
    }

    /// The original display spelling.
    pub fn display(&self) -> &str {
        &self.display
    }

    /// The verified normalized UTS #39 skeleton, compared bytewise.
    pub fn skeleton(&self) -> &str {
        &self.skeleton
    }
}

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Handle([redacted])")
    }
}

/// The community role. Authorization to assign a role belongs to the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Platform operator in this community's scope.
    Root,
    /// Community administrator.
    Admin,
    /// Ordinary member.
    Member,
}

/// A calendar month, without a day, time, or timezone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct YearMonth {
    year: u16,
    month: u8,
}

impl YearMonth {
    /// Construct a month in years 1–9999.
    pub fn new(year: u16, month: u8) -> Result<Self> {
        if !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
            return Err(Error::InvalidInput);
        }
        Ok(Self { year, month })
    }

    /// Calendar year.
    pub fn year(self) -> u16 {
        self.year
    }

    /// Calendar month, 1–12.
    pub fn month(self) -> u8 {
        self.month
    }

    /// Add calendar months, returning an error instead of overflowing.
    pub fn checked_add(self, months: u32) -> Result<Self> {
        let ordinal = (u32::from(self.year) - 1) * 12 + u32::from(self.month) - 1;
        let ordinal = ordinal.checked_add(months).ok_or(Error::DateOverflow)?;
        if ordinal >= 9999 * 12 {
            return Err(Error::DateOverflow);
        }
        Self::new((ordinal / 12 + 1) as u16, (ordinal % 12 + 1) as u8)
    }

    pub(crate) fn at(now: DateTime<Utc>) -> Result<Self> {
        let year = u16::try_from(now.year()).map_err(|_| Error::InvalidInput)?;
        Self::new(year, now.month() as u8)
    }
}

impl fmt::Display for YearMonth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

impl FromStr for YearMonth {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let bytes = value.as_bytes();
        if bytes.len() != 7
            || bytes[4] != b'-'
            || !bytes[..4].iter().chain(&bytes[5..]).all(u8::is_ascii_digit)
        {
            return Err(Error::InvalidInput);
        }
        Self::new(
            value[..4].parse().map_err(|_| Error::InvalidInput)?,
            value[5..].parse().map_err(|_| Error::InvalidInput)?,
        )
    }
}

/// Calendar-month retention after a lease ends or a handle changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleasePeriod {
    months: u32,
}

impl ReleasePeriod {
    /// Set a nonnegative retention period; dates are checked when used.
    pub const fn months(months: u32) -> Self {
        Self { months }
    }

    /// The configured number of calendar months.
    pub const fn as_months(self) -> u32 {
        self.months
    }

    pub(crate) fn after(self, month: YearMonth) -> Result<YearMonth> {
        month.checked_add(self.months.checked_add(1).ok_or(Error::DateOverflow)?)
    }
}

impl Default for ReleasePeriod {
    fn default() -> Self {
        Self::months(24)
    }
}

/// The permanent community member record, with no activity timestamps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// Community pseudonym.
    pub id: MemberId,
    /// Current handle, absent after release.
    pub handle: Option<Handle>,
    /// Caller-authorized role.
    pub role: Role,
    /// Lease valid through the end of this month.
    pub lease_end: YearMonth,
}

/// Input to admission; an exact retry returns the existing member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admission {
    /// Community pseudonym.
    pub id: MemberId,
    /// Must match the member's live reservation exactly.
    pub handle: Handle,
    /// Caller-authorized initial role.
    pub role: Role,
    /// Initial lease end, not earlier than the current month.
    pub lease_end: YearMonth,
}

/// A pending enrolment's temporary handle claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reservation {
    /// Community pseudonym of the pending member.
    pub member_id: MemberId,
    /// Validated handle supplied by the caller.
    pub handle: Handle,
    /// Exclusive UTC expiry in whole seconds.
    pub expires_at: DateTime<Utc>,
}

/// Counts from one bounded maintenance batch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Released {
    /// Expired pending reservations removed.
    pub reservations: usize,
    /// Member handles released; member IDs, roles and leases are retained.
    pub handles: usize,
    /// Expired old-handle blocks removed.
    pub blocks: usize,
}
