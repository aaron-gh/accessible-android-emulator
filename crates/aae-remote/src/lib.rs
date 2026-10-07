//! Server for AAE Remote: device management, device audio and vibration to
//! the phone, keys, touches and microphone to the device.

pub mod daemon;
pub mod discovery;
mod json;
mod media;
pub mod security;
pub mod server;
mod tools;
mod vibration;

pub use server::{DEFAULT_PORT, Server};
