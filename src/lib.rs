//! quarry — see every server running on this machine, and whose repo it came
//! from.
//!
//! The crate is a library first so that the discovery pipeline, the state
//! machine and the rendering can all be tested without a terminal or a machine
//! full of servers underneath them. The binary is a thin shell over it.

#![deny(unsafe_code)]
#![warn(clippy::all)]

pub mod app;
pub mod config;
pub mod darwin;
pub mod diag;
pub mod docker;
pub mod doctor;
pub mod engine;
pub mod exec;
pub mod handshake;
pub mod keys;
pub mod lsof;
pub mod model;
pub mod probe;
pub mod procs;
pub mod repo;
pub mod runtime;
pub mod signature;
pub mod source;
pub mod term;
pub mod testkit;
pub mod theme;
pub mod ui;
