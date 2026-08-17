//! comar-api hata tipleri.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("D-Bus bağlantı hatası: {0}")]
    Connection(String),

    #[error("D-Bus çağrı hatası: {0}")]
    Call(String),

    #[error("PolicyKit hatası: {0}")]
    PolicyKit(String),

    #[error("zbus hatası: {0}")]
    Zbus(#[from] zbus::Error),
}

pub type Result<T> = std::result::Result<T, ApiError>;
