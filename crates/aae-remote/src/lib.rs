//! Serves AAE's devices to AAE's Android app over the network, so a phone can
//! use devices running on a Mac or PC: list, start and manage them, hear them,
//! feel their vibrations, and send them keys and touches.

pub mod daemon;
pub mod discovery;
mod json;
mod media;
pub mod security;
pub mod server;
mod tools;
mod vibration;

pub use server::{DEFAULT_PORT, Server};
