//! Pure logic of the `messaging-api` channel.
//!
//! Everything here is host-neutral: state, time, randomness and outbound HTTP
//! arrive through the traits in [`env`] and [`store`], so the WASM component is
//! a thin adapter and every rule is testable natively.

pub mod config;
pub mod env;
pub mod event;
pub mod log;
pub mod send;
pub mod store;
pub mod webhook;

#[cfg(test)]
mod fixtures_tests;
#[cfg(test)]
mod log_tests;
#[cfg(test)]
mod send_tests;
