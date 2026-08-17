//! XML model yükleyici entegrasyon testleri.
//!
//! Gerçek model dosyalarıyla `db.rs`'in tüm davranışını doğrular.

use comar_daemon::db::{
    load_models, signature_in, signature_out, validate_model_member, MemberType,
};
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/models")
}

/// Fixture dizini yoksa (CI'da kaynak dışı ortam) testi atla
macro_rules! require_fixtures {
    () => {
        if !fixtures_dir().exists() {
            eprintln!("Fixtures bulunamadı, test atlandı.");
            return;
        }
    };
}

const PREFIX: &str = "tr.org.luppo.comar";

// ── Yükleme testleri ──────────────────────────────────────────────────────────

#[test]
fn loads_all_20_models() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    assert_eq!(reg.len(), 20, "20 model arayüzü bekleniyor, {} bulundu", reg.len());
}

#[test]
fn all_expected_models_present() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    let expected = [
        "Boot.Loader", "Boot.Modules", "Core", "Disk.Manager",
        "Net.Filter", "Net.Link", "Net.Share", "Net.Stack",
        "Network.Firewall", "Network.Link", "Network.Stack",
        "System.Manager", "System.Package", "System.PackageHandler",
        "System.Service", "System.Settings", "Time.Clock",
        "User.Manager", "Xorg.Display", "Xorg.Driver",
    ];
    for model in &expected {
        assert!(reg.contains_key(*model), "Model eksik: {}", model);
    }
}

// ── System.Service testleri ───────────────────────────────────────────────────

#[test]
fn system_service_start_is_noreply() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    let def = &reg["System.Service"]["start"];
    assert!(def.no_reply,   "start no_reply olmalı");
    assert!(def.args_out.is_empty(), "start'ın çıkış argümanı olmamalı");
    assert_eq!(def.action_id, "tr.org.luppo.comar.system.service.set");
}

#[test]
fn system_service_info_has_three_out_args() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    let def = &reg["System.Service"]["info"];
    assert_eq!(def.args_out, vec!["s", "s", "s"], "info() üç string döndürmeli");
    assert_eq!(def.signature_out(), "sss");
    assert!(def.action_id.is_empty(), "info() action_id boş olmalı");
}

#[test]
fn system_service_changed_is_signal() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    let def = &reg["System.Service"]["Changed"];
    assert_eq!(def.kind, MemberType::Signal);
    assert_eq!(def.args_out, vec!["s", "s"]);
}

// ── Net.Link testleri ─────────────────────────────────────────────────────────

#[test]
fn net_link_setstate_noreply_with_action_id() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    let def = &reg["Net.Link"]["setState"];
    assert!(def.no_reply);
    assert_eq!(def.args_in, vec!["s", "s"]);
    assert_eq!(def.signature_in(), "ss");
    assert_eq!(def.action_id, "tr.org.luppo.comar.net.link.setstate");
}

#[test]
fn net_link_has_three_signals() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    let signals: Vec<_> = reg["Net.Link"]
        .values()
        .filter(|d| d.kind == MemberType::Signal)
        .collect();
    assert_eq!(signals.len(), 3, "Net.Link 3 sinyale sahip olmalı");
}

// ── System.Manager testleri ───────────────────────────────────────────────────

#[test]
fn system_manager_self_closing_methods_registered() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    assert!(
        reg["System.Manager"].contains_key("updateAllRepositories"),
        "self-closing updateAllRepositories kayıtlı olmalı"
    );
    assert!(
        reg["System.Manager"].contains_key("takeSnapshot"),
        "self-closing takeSnapshot kayıtlı olmalı"
    );
}

#[test]
fn system_manager_has_11_signals() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    let signals: Vec<_> = reg["System.Manager"]
        .values()
        .filter(|d| d.kind == MemberType::Signal)
        .collect();
    assert_eq!(signals.len(), 11);
}

// ── Boot.Loader testleri ──────────────────────────────────────────────────────

#[test]
fn boot_loader_listsystems_empty_action_id() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    assert_eq!(
        reg["Boot.Loader"]["listSystems"].action_id,
        "",
        "listSystems action_id boş olmalı (açık erişim)"
    );
    assert_eq!(
        reg["Boot.Loader"]["listSystems"].signature_out(),
        "a{s(sasas)}"
    );
}

// ── User.Manager testleri ─────────────────────────────────────────────────────

#[test]
fn user_manager_adduser_has_many_in_args() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    let def = &reg["User.Manager"]["addUser"];
    assert_eq!(def.args_in.len(), 9, "addUser 9 giriş argümanına sahip olmalı");
    assert_eq!(def.args_out, vec!["i"]);
    assert_eq!(def.signature_out(), "i");
}

// ── validate_model_member testleri ────────────────────────────────────────────

#[test]
fn validate_existing_method() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    assert!(validate_model_member(&reg, "System.Service", "start",   &MemberType::Method));
    assert!(validate_model_member(&reg, "System.Service", "stop",    &MemberType::Method));
    assert!(validate_model_member(&reg, "System.Service", "reload",  &MemberType::Method));
    assert!(validate_model_member(&reg, "System.Service", "info",    &MemberType::Method));
}

#[test]
fn validate_signal_vs_method_distinction() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    // Changed sinyal, method değil
    assert!( validate_model_member(&reg, "System.Service", "Changed", &MemberType::Signal));
    assert!(!validate_model_member(&reg, "System.Service", "Changed", &MemberType::Method));
    // start method, sinyal değil
    assert!( validate_model_member(&reg, "System.Service", "start",   &MemberType::Method));
    assert!(!validate_model_member(&reg, "System.Service", "start",   &MemberType::Signal));
}

#[test]
fn validate_nonexistent_member_returns_false() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    assert!(!validate_model_member(&reg, "System.Service", "ghostMethod", &MemberType::Method));
    assert!(!validate_model_member(&reg, "NoSuchModel",    "anything",    &MemberType::Method));
}

// ── signature_in / signature_out testleri ────────────────────────────────────

#[test]
fn signature_helpers_correct() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();

    // start: giriş yok, çıkış yok (no_reply)
    assert_eq!(signature_in (&reg, "System.Service", "start"), Some("".into()));
    assert_eq!(signature_out(&reg, "System.Service", "start"), Some("".into()));

    // info: giriş yok, çıkış "sss"
    assert_eq!(signature_in (&reg, "System.Service", "info"), Some("".into()));
    assert_eq!(signature_out(&reg, "System.Service", "info"), Some("sss".into()));

    // setState: giriş "s", çıkış yok (no_reply)
    assert_eq!(signature_in (&reg, "System.Service", "setState"), Some("s".into()));
    assert_eq!(signature_out(&reg, "System.Service", "setState"), Some("".into()));
}

#[test]
fn signature_returns_none_for_unknown() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    assert_eq!(signature_out(&reg, "System.Service", "nosuchmethod"), None);
    assert_eq!(signature_out(&reg, "NoSuchModel",    "anymethod"),    None);
}

// ── Core model testleri ───────────────────────────────────────────────────────

#[test]
fn core_model_has_required_methods() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    let core_methods = ["register", "remove", "listModelApplications",
                        "listApplicationModels", "introspect", "cancel",
                        "listRunning", "setLocale"];
    for m in &core_methods {
        assert!(
            reg["Core"].contains_key(*m),
            "Core modeli '{}' metoduna sahip olmalı", m
        );
    }
}

#[test]
fn core_register_has_restricted_action_id() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    assert_eq!(
        reg["Core"]["register"].action_id,
        "tr.org.luppo.comar.register"
    );
}

// ── Xorg.Display — birden fazla arayüz dosyasında ─────────────────────────────

#[test]
fn xorg_display_syncconfigs_self_closing_with_access_label() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    let def = &reg["Xorg.Display"]["syncConfigs"];
    assert_eq!(def.action_id, "tr.org.luppo.comar.xorg.display.set");
    assert!(def.args_in.is_empty());
    assert!(def.args_out.is_empty());
}

#[test]
fn xorg_driver_enable_disable_self_closing() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    assert!(reg["Xorg.Driver"].contains_key("enable"));
    assert!(reg["Xorg.Driver"].contains_key("disable"));
}

// ── Çoklu arayüz (Net.Link.xml 4 arayüz içeriyor) ────────────────────────────

#[test]
fn net_link_xml_contains_multiple_interfaces() {
    require_fixtures!();
    let reg = load_models(&fixtures_dir(), PREFIX).unwrap();
    // Net.Link.xml: Net.Link, Net.Share, Net.Stack, Net.Filter
    assert!(reg.contains_key("Net.Link"));
    assert!(reg.contains_key("Net.Share"));
    assert!(reg.contains_key("Net.Stack"));
    assert!(reg.contains_key("Net.Filter"));
}
