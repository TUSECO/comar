//! Ortak tipler.

use zvariant::OwnedValue;

/// D-Bus metod çağrısı tanımlayıcısı.
pub struct ComarCall {
    pub app:    Option<String>,
    pub model:  String,
    pub method: String,
}

/// Sinyal işleyici callback tipi.
pub type SignalHandler = Box<dyn Fn(&str, &str, &[OwnedValue]) + Send + Sync>;
