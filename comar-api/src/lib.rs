//! ÇOMAR istemci API kütüphanesi.
//!
//! comar-api Python paketinin Rust karşılığı.
//! Faz 8'de tam implementasyon yapılacak.

pub mod error;
pub mod link;
pub mod types;

pub use error::ApiError;
pub use link::ComarLink;
pub use types::{ComarCall, SignalHandler};
