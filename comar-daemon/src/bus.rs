//! D-Bus mesaj yardımcıları — bus.c karşılığı.
//!
//! Mesaj gönderme, hata cevaplama ve sinyal yayınlama
//! fonksiyonları burada toplanmıştır.

use zbus::message::Message;

// ── Yol ayrıştırma ─────────────────────────────────────────────────────────────

/// D-Bus yolundan uygulama adını çıkarır.
///
/// `/package/apache` → `Some("apache")`
/// `/`              → `None`
///
/// `strsub(path, strlen("/package/"), strlen(path))` karşılığı.
pub fn app_from_path(path: &str) -> Option<&str> {
    path.strip_prefix("/package/").filter(|s| !s.is_empty())
}

/// D-Bus arayüz adından model adını çıkarır.
///
/// `tr.org.luppo.comar.Net.Link` → `Some("Net.Link")`
/// `tr.org.luppo.comar`          → `None`
///
/// `strsub(iface, strlen(config_interface) + 1, strlen(iface))` karşılığı.
pub fn model_from_interface<'a>(iface: &'a str, prefix: &str) -> Option<&'a str> {
    iface
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('.'))
        .filter(|s| !s.is_empty())
}

// ── Mesaj ayrıştırma ──────────────────────────────────────────────────────────

/// Gelen D-Bus mesajının yönlendirme bilgilerini barındırır.
#[derive(Debug, Clone)]
pub enum Dispatch {
    /// org.freedesktop.DBus.Introspectable.Introspect
    Introspect { path: String },

    /// tr.org.luppo.comar → Core metodları (path="/")
    /// setLocale, cancel, listRunning, register, remove, …
    Core {
        method: String,
        sender: String,
    },

    /// tr.org.luppo.comar.<Model> → paket betiği
    Model {
        app:    String,
        model:  String,
        method: String,
        sender: String,
        path:   String,
    },

    /// Bilinmeyen metot
    Unknown,
}

/// Mesajdan `Dispatch` türetir — `filter_func` + `handle_message` birleşimi.
pub fn dispatch_message(msg: &Message, interface_prefix: &str) -> Dispatch {
    let method = msg.header().member().map(|m| m.to_string()).unwrap_or_default();
    let iface  = msg.header().interface().map(|i| i.to_string()).unwrap_or_default();
    let path   = msg.header().path().map(|p| p.to_string()).unwrap_or_default();
    let sender = msg.header().sender().map(|s| s.to_string()).unwrap_or_default();

    // Introspection
    if iface == "org.freedesktop.DBus.Introspectable" && method == "Introspect" {
        return Dispatch::Introspect { path };
    }

    // Core arayüzü (prefix'in kendisi): tr.org.luppo.comar
    if iface == interface_prefix {
        return Dispatch::Core { method, sender };
    }

    // Model arayüzü: tr.org.luppo.comar.Net.Link  →  model = "Net.Link"
    if let Some(model) = model_from_interface(&iface, interface_prefix) {
        let app = app_from_path(&path)
            .unwrap_or("")
            .to_string();
        return Dispatch::Model {
            app,
            model: model.to_string(),
            method,
            sender,
            path,
        };
    }

    Dispatch::Unknown
}

// ── D-Bus hata adı üretimi ────────────────────────────────────────────────────

/// `bus_reply_error()` karşılığı.
/// `tr.org.luppo.comar.ErrorName` biçiminde D-Bus hata adı döndürür.
#[allow(dead_code)]
pub fn make_error_name(prefix: &str, short_name: &str) -> String {
    format!("{}.{}", prefix, short_name)
}

// ── Testler ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = "tr.org.luppo.comar";

    #[test]
    fn app_from_path_package() {
        assert_eq!(app_from_path("/package/apache"), Some("apache"));
        assert_eq!(app_from_path("/package/NetworkManager"), Some("NetworkManager"));
    }

    #[test]
    fn app_from_path_root() {
        assert_eq!(app_from_path("/"), None);
        assert_eq!(app_from_path("/package/"), None);
    }

    #[test]
    fn model_from_interface_ok() {
        assert_eq!(
            model_from_interface("tr.org.luppo.comar.Net.Link", PREFIX),
            Some("Net.Link")
        );
        assert_eq!(
            model_from_interface("tr.org.luppo.comar.User.Manager", PREFIX),
            Some("User.Manager")
        );
        assert_eq!(
            model_from_interface("tr.org.luppo.comar.System.Service", PREFIX),
            Some("System.Service")
        );
    }

    #[test]
    fn model_from_interface_core() {
        // Core arayüzü: prefix'in kendisi → None
        assert_eq!(model_from_interface(PREFIX, PREFIX), None);
    }

    #[test]
    fn model_from_interface_unrelated() {
        assert_eq!(
            model_from_interface("org.freedesktop.DBus", PREFIX),
            None
        );
    }

    #[test]
    fn make_error_name_format() {
        assert_eq!(
            make_error_name(PREFIX, "PolicyKit"),
            "tr.org.luppo.comar.PolicyKit"
        );
        assert_eq!(
            make_error_name(PREFIX, "Internal"),
            "tr.org.luppo.comar.Internal"
        );
    }
}
