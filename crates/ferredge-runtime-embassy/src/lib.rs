//! Embassy adapter scaffold.
//!
//! Embassy uses a different executor and ownership model than std-first runtimes, so this crate
//! is intentionally a scaffold for now. The core runtime and net traits already live in
//! `ferredge-core`; this crate will host the embassy-specific implementations.

#![no_std]

pub struct EmbassyRuntime;

pub struct EmbassyNet;
