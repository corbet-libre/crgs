//! The crate's integration tests in one binary.
//!
//! Every former `tests/<name>.rs` is a module here, so a warm edit loop links one test
//! executable instead of one per file. The test names keep their module prefix
//! (`<name>::<test>`).
mod admission_transaction;
mod database;
mod first_transaction;
mod member_transaction;
mod register;
mod renewal_transaction;
mod role_guard;
