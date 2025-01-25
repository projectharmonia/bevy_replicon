//! A simple transport intended only for examples.
//! This transport does not implement any reliability or security features.
//! DO NOT USE in a real project

#[cfg(feature = "client")]
mod client;
#[cfg(feature = "server")]
mod server;

#[cfg(feature = "client")]
pub use client::*;
#[cfg(feature = "server")]
pub use server::*;
