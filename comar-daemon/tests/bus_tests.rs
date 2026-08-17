//! D-Bus mesaj yönlendirme testleri.
//!
//! `bus.rs`'deki `dispatch_message`, `app_from_path`, `model_from_interface`
//! fonksiyonlarını doğrular.

use comar_daemon::bus::{app_from_path, model_from_interface};

const PREFIX: &str = "tr.org.luppo.comar";

// ── app_from_path ─────────────────────────────────────────────────────────────

#[test]
fn app_from_path_standard() {
    assert_eq!(app_from_path("/package/apache"),         Some("apache"));
    assert_eq!(app_from_path("/package/NetworkManager"), Some("NetworkManager"));
    assert_eq!(app_from_path("/package/net_link"),       Some("net_link"));
}

#[test]
fn app_from_path_empty_or_root() {
    assert_eq!(app_from_path("/"),         None);
    assert_eq!(app_from_path("/package"),  None);
    assert_eq!(app_from_path("/package/"), None);
}

#[test]
fn app_from_path_nested() {
    // Sadece tek seviye desteklenir
    assert_eq!(app_from_path("/package/some/nested"), Some("some/nested"));
}

// ── model_from_interface ──────────────────────────────────────────────────────

#[test]
fn model_from_interface_standard_models() {
    assert_eq!(model_from_interface("tr.org.luppo.comar.Net.Link",       PREFIX), Some("Net.Link"));
    assert_eq!(model_from_interface("tr.org.luppo.comar.System.Service", PREFIX), Some("System.Service"));
    assert_eq!(model_from_interface("tr.org.luppo.comar.User.Manager",   PREFIX), Some("User.Manager"));
    assert_eq!(model_from_interface("tr.org.luppo.comar.Time.Clock",     PREFIX), Some("Time.Clock"));
    assert_eq!(model_from_interface("tr.org.luppo.comar.Xorg.Display",   PREFIX), Some("Xorg.Display"));
}

#[test]
fn model_from_interface_core_returns_none() {
    // Core arayüzü prefix'in kendisidir → None
    assert_eq!(model_from_interface(PREFIX, PREFIX), None);
}

#[test]
fn model_from_interface_unrelated() {
    assert_eq!(model_from_interface("org.freedesktop.DBus",          PREFIX), None);
    assert_eq!(model_from_interface("org.freedesktop.NetworkManager",PREFIX), None);
    assert_eq!(model_from_interface("",                               PREFIX), None);
}

#[test]
fn model_from_interface_partial_match_rejected() {
    // "tr.org.luppo.comarX.Net.Link" eşleşmemeli
    assert_eq!(
        model_from_interface("tr.org.luppo.comarX.Net.Link", PREFIX),
        None
    );
}

// ── make_error_name ───────────────────────────────────────────────────────────

#[test]
fn make_error_name_format() {
    use comar_daemon::bus::make_error_name;
    assert_eq!(
        make_error_name(PREFIX, "PolicyKit"),
        "tr.org.luppo.comar.PolicyKit"
    );
    assert_eq!(
        make_error_name(PREFIX, "Internal"),
        "tr.org.luppo.comar.Internal"
    );
    assert_eq!(
        make_error_name(PREFIX, "Missing"),
        "tr.org.luppo.comar.Missing"
    );
}
