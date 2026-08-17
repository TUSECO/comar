//! ÇOMAR hata tipleri.
//!
//! C kodundaki PyExc_COMAR_* + PyExc_DBus + PyExc_PolicyKit karşılıkları.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ComarError {
    // ── Python script hataları ─────────────────────────────────────────────────
    /// İç ÇOMAR hatası (beklenmedik durum)
    #[error("Comar.Internal: {0}")]
    Internal(String),

    /// Geçersiz argüman veya model/method adı
    #[error("Comar.Invalid: {0}")]
    #[allow(dead_code)]
    Invalid(String),

    /// Python betiği yüklenirken / çalıştırılırken hata
    #[error("Comar.Script: {0}")]
    Script(String),

    /// Betik bulunamadı veya fonksiyon tanımlı değil
    #[error("Comar.Missing: {0}")]
    Missing(String),

    // ── D-Bus hataları ─────────────────────────────────────────────────────────
    /// D-Bus iletişim hatası
    #[error("Comar.DBus: {0}")]
    DBus(String),

    // ── PolicyKit hataları ─────────────────────────────────────────────────────
    /// PolicyKit yetkilendirmesi reddedildi (action_id taşır)
    #[error("Comar.PolicyKit: {0}")]
    #[allow(dead_code)]
    PolicyKit(String),

    /// PolicyKit sorgusunda hata oluştu
    #[error("Comar.PolicyKit: error")]
    #[allow(dead_code)]
    PolicyKitError,

    // ── Altyapı hataları ──────────────────────────────────────────────────────
    /// Dosya sistemi hatası
    #[error("I/O hatası: {0}")]
    Io(#[from] std::io::Error),

    /// XML ayrıştırma hatası
    #[error("XML ayrıştırma hatası: {0}")]
    Xml(String),

    /// zbus D-Bus hatası
    #[error("zbus hatası: {0}")]
    Zbus(#[from] zbus::Error),

    /// Python (PyO3) hatası
    #[error("Python hatası: {0}")]
    Python(String),
}

/// D-Bus hata adı üretir: "tr.org.luppo.comar.ErrorName"
impl ComarError {
    #[allow(dead_code)]
    pub fn dbus_error_name(&self, interface_prefix: &str) -> String {
        let suffix = match self {
            Self::Internal(_)     => "Internal",
            Self::Invalid(_)      => "Invalid",
            Self::Script(_)       => "Script",
            Self::Missing(_)      => "Missing",
            Self::DBus(_)         => "DBus",
            Self::PolicyKit(_)    => "PolicyKit",
            Self::PolicyKitError  => "PolicyKit",
            Self::Zbus(_)         => "DBus",
            Self::Python(_)       => "Script",
            Self::Xml(_)          => "Internal",
            Self::Io(_)           => "Internal",
        };
        format!("{}.{}", interface_prefix, suffix)
    }

    /// Hata mesajını döndürür (D-Bus reply için)
    #[allow(dead_code)]
    pub fn message(&self) -> &str {
        match self {
            Self::Internal(m)    => m,
            Self::Invalid(m)     => m,
            Self::Script(m)      => m,
            Self::Missing(m)     => m,
            Self::DBus(m)        => m,
            Self::PolicyKit(m)   => m,
            Self::PolicyKitError => "error",
            Self::Io(_)          => "I/O error",
            Self::Xml(m)         => m,
            Self::Zbus(_)        => "D-Bus error",
            Self::Python(m)      => m,
        }
    }
}

// PyO3 hatalarını ComarError'a dönüştür
impl From<pyo3::PyErr> for ComarError {
    fn from(err: pyo3::PyErr) -> Self {
        pyo3::Python::with_gil(|py| {
            // Python hatasının tipini ve mesajını al
            let err_msg = format!("{}", err.value_bound(py));
            ComarError::Python(err_msg)
        })
    }
}

pub type Result<T> = std::result::Result<T, ComarError>;

// ── Testler ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = "tr.org.luppo.comar";

    #[test]
    fn dbus_error_name_internal() {
        let err = ComarError::Internal("test".into());
        assert_eq!(err.dbus_error_name(PREFIX), "tr.org.luppo.comar.Internal");
    }

    #[test]
    fn dbus_error_name_policykit() {
        let err = ComarError::PolicyKit("tr.org.luppo.comar.net.link.set".into());
        assert_eq!(err.dbus_error_name(PREFIX), "tr.org.luppo.comar.PolicyKit");
    }

    #[test]
    fn error_display() {
        let err = ComarError::Missing("start not defined".into());
        assert_eq!(err.to_string(), "Comar.Missing: start not defined");
    }
}
