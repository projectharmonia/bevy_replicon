//! A simple transport intended only for examples.
//! This transport does not implement any reliability or security features.
//! DO NOT USE in a real project

mod client;
pub use client::*;

mod server;
pub use server::*;
