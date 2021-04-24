#![allow(clippy::integer_arithmetic)]
pub mod counter;
pub mod datapoint;
mod metrics;
pub use crate::metrics::{flush, query, set_host_id, set_panic_hook, submit};


use std::sync::atomic::AtomicU64;
lazy_static::lazy_static! {
    pub static ref LAST_VOTE_SLOT: AtomicU64 = {
        std::sync::atomic::AtomicU64::default()
    };
}
