//! comar-daemon kütüphane arayüzü.
//!
//! Bu modül yalnızca entegrasyon testleri için iç yapıyı dışa açar.
//! Normal kullanımda `comar` binary'si kullanılır.

rust_i18n::i18n!("../../locales", fallback = "tr");

pub mod bus;
pub mod config;
pub mod core;
pub mod db;
pub mod error;
pub mod logging;
pub mod policy;
pub mod process;
pub mod script;
pub mod declarative;
pub mod services;
pub mod signal_bus;
