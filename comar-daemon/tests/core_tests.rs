//! Core arayüz operasyonları testleri.
//!
//! `register`, `remove`, `list_model_applications`, `list_application_models`,
//! `introspect` ve locale yönetimi fonksiyonlarını doğrular.

use comar_daemon::{
    config::{Config, DBUS_SERVER_ADDRESS, DBUS_SERVICE_NAME, DEFAULT_IDLE_TIMEOUT},
    core::{
        introspect, list_application_models, list_model_applications,
        new_locale_map, register, remove, sender_language,
    },
    db::load_models,
};
use std::{fs, path::PathBuf};

// ── Test altyapısı ────────────────────────────────────────────────────────────

fn test_config(tmp: &std::path::Path) -> Config {
    Config {
        service_name:   DBUS_SERVICE_NAME.into(),
        dir_data:       tmp.to_path_buf(),
        dir_log:        tmp.join("log"),
        debug:          false,
        ignore_missing: false,
        print_mode:     false,
        server_address: DBUS_SERVER_ADDRESS.into(),
        timeout:        DEFAULT_IDLE_TIMEOUT,
    }
}

fn setup_tmp() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = test_config(tmp.path());
    fs::create_dir_all(cfg.dir_scripts()).unwrap();
    fs::create_dir_all(cfg.dir_apps()).unwrap();
    fs::create_dir_all(cfg.dir_models()).unwrap();
    (tmp, cfg)
}

fn write_script(cfg: &Config, app: &str, _model: &str, content: &str) -> PathBuf {
    let script_path = cfg.dir_data.join(format!("{}.py", app));
    fs::write(&script_path, content).unwrap();
    script_path
}

// ── register / remove ─────────────────────────────────────────────────────────

#[test]
fn register_creates_script_and_marker() {
    let (_tmp, cfg) = setup_tmp();
    let script = write_script(&cfg, "apache", "System.Service", "def start(): pass");

    register(&cfg, "apache", "System.Service", script.to_str().unwrap()).unwrap();

    // scripts/System.Service/apache.py oluştu mu?
    assert!(
        cfg.dir_scripts().join("System.Service").join("apache.py").exists(),
        "betik dosyası oluşturulmalı"
    );
    // apps/apache/System.Service işaret dosyası oluştu mu?
    assert!(
        cfg.dir_apps().join("apache").join("System.Service").exists(),
        "işaret dosyası oluşturulmalı"
    );
}

#[test]
fn register_multiple_models_for_same_app() {
    let (_tmp, cfg) = setup_tmp();
    let script = write_script(&cfg, "apache", "multi", "");

    register(&cfg, "apache", "System.Service", script.to_str().unwrap()).unwrap();
    register(&cfg, "apache", "System.Package", script.to_str().unwrap()).unwrap();

    assert!(cfg.dir_apps().join("apache").join("System.Service").exists());
    assert!(cfg.dir_apps().join("apache").join("System.Package").exists());
}

#[test]
fn register_multiple_apps_for_same_model() {
    let (_tmp, cfg) = setup_tmp();
    let script = write_script(&cfg, "multi", "System.Service", "");

    register(&cfg, "apache", "System.Service", script.to_str().unwrap()).unwrap();
    register(&cfg, "nginx",  "System.Service", script.to_str().unwrap()).unwrap();

    assert!(cfg.dir_scripts().join("System.Service").join("apache.py").exists());
    assert!(cfg.dir_scripts().join("System.Service").join("nginx.py").exists());
}

#[test]
fn remove_cleans_all_model_scripts() {
    let (_tmp, cfg) = setup_tmp();
    let script = write_script(&cfg, "apache", "multi", "");

    register(&cfg, "apache", "System.Service", script.to_str().unwrap()).unwrap();
    register(&cfg, "apache", "System.Package", script.to_str().unwrap()).unwrap();

    remove(&cfg, "apache").unwrap();

    assert!(!cfg.dir_apps().join("apache").exists(),                              "apps/apache/ silinmeli");
    assert!(!cfg.dir_scripts().join("System.Service").join("apache.py").exists(), "betik silinmeli");
    assert!(!cfg.dir_scripts().join("System.Package").join("apache.py").exists(), "betik silinmeli");
}

#[test]
fn remove_nonexistent_app_is_noop() {
    let (_tmp, cfg) = setup_tmp();
    // Hata vermemeli
    remove(&cfg, "nonexistent").unwrap();
}

#[test]
fn remove_does_not_affect_other_apps() {
    let (_tmp, cfg) = setup_tmp();
    let script = write_script(&cfg, "multi", "multi", "");

    register(&cfg, "apache", "System.Service", script.to_str().unwrap()).unwrap();
    register(&cfg, "nginx",  "System.Service", script.to_str().unwrap()).unwrap();

    remove(&cfg, "apache").unwrap();

    // nginx etkilenmemeli
    assert!(cfg.dir_scripts().join("System.Service").join("nginx.py").exists());
    assert!(cfg.dir_apps().join("nginx").join("System.Service").exists());
}

// ── list_model_applications ───────────────────────────────────────────────────

#[test]
fn list_model_apps_empty_when_no_scripts() {
    let (_tmp, cfg) = setup_tmp();
    let apps = list_model_applications(&cfg, "System.Service").unwrap();
    assert!(apps.is_empty());
}

#[test]
fn list_model_apps_returns_registered() {
    let (_tmp, cfg) = setup_tmp();
    let script = write_script(&cfg, "multi", "multi", "");

    register(&cfg, "apache", "System.Service", script.to_str().unwrap()).unwrap();
    register(&cfg, "nginx",  "System.Service", script.to_str().unwrap()).unwrap();
    register(&cfg, "vsftpd", "System.Service", script.to_str().unwrap()).unwrap();

    let mut apps = list_model_applications(&cfg, "System.Service").unwrap();
    apps.sort();
    assert_eq!(apps, vec!["apache", "nginx", "vsftpd"]);
}

#[test]
fn list_model_apps_after_remove() {
    let (_tmp, cfg) = setup_tmp();
    let script = write_script(&cfg, "multi", "multi", "");

    register(&cfg, "apache", "System.Service", script.to_str().unwrap()).unwrap();
    register(&cfg, "nginx",  "System.Service", script.to_str().unwrap()).unwrap();

    remove(&cfg, "apache").unwrap();

    let apps = list_model_applications(&cfg, "System.Service").unwrap();
    assert_eq!(apps, vec!["nginx"]);
}

// ── list_application_models ───────────────────────────────────────────────────

#[test]
fn list_app_models_empty_when_not_registered() {
    let (_tmp, cfg) = setup_tmp();
    let models = list_application_models(&cfg, "apache").unwrap();
    assert!(models.is_empty());
}

#[test]
fn list_app_models_returns_all() {
    let (_tmp, cfg) = setup_tmp();
    let script = write_script(&cfg, "multi", "multi", "");

    register(&cfg, "apache", "System.Service", script.to_str().unwrap()).unwrap();
    register(&cfg, "apache", "System.Package", script.to_str().unwrap()).unwrap();

    let mut models = list_application_models(&cfg, "apache").unwrap();
    models.sort();
    assert_eq!(models, vec!["System.Package", "System.Service"]);
}

// ── introspect ────────────────────────────────────────────────────────────────

fn load_test_registry(_cfg: &Config) -> std::sync::Arc<comar_daemon::db::ModelRegistry> {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/models");
    if fixtures.exists() {
        std::sync::Arc::new(
            load_models(&fixtures, DBUS_SERVICE_NAME).unwrap()
        )
    } else {
        std::sync::Arc::new(comar_daemon::db::ModelRegistry::new())
    }
}

#[test]
fn introspect_root_contains_core_and_package_node() {
    let (_tmp, cfg) = setup_tmp();
    let registry = load_test_registry(&cfg);

    let xml = introspect(&cfg, &registry, "/").unwrap();

    assert!(xml.contains("<node"), "XML node içermeli");
    assert!(xml.contains(&format!("name=\"{}\"", DBUS_SERVICE_NAME)),
            "Core arayüzü bulunmalı");
    assert!(xml.contains("<node name=\"package\""),
            "package alt node'u bulunmalı");
}

#[test]
fn introspect_package_lists_registered_apps() {
    let (_tmp, cfg) = setup_tmp();
    let registry = load_test_registry(&cfg);
    let script = write_script(&cfg, "multi", "multi", "");

    register(&cfg, "apache", "System.Service", script.to_str().unwrap()).unwrap();
    register(&cfg, "nginx",  "System.Service", script.to_str().unwrap()).unwrap();

    let xml = introspect(&cfg, &registry, "/package").unwrap();

    assert!(xml.contains("name=\"apache\""), "apache node'u bulunmalı");
    assert!(xml.contains("name=\"nginx\""),  "nginx node'u bulunmalı");
}

#[test]
fn introspect_package_app_lists_interfaces() {
    let (_tmp, cfg) = setup_tmp();
    let registry = load_test_registry(&cfg);
    let script = write_script(&cfg, "multi", "multi", "");

    register(&cfg, "apache", "System.Service", script.to_str().unwrap()).unwrap();

    let xml = introspect(&cfg, &registry, "/package/apache").unwrap();

    assert!(
        xml.contains(&format!("{}.System.Service", DBUS_SERVICE_NAME)),
        "System.Service arayüzü bulunmalı"
    );
}

#[test]
fn introspect_unknown_path_returns_empty_node() {
    let (_tmp, cfg) = setup_tmp();
    let registry = load_test_registry(&cfg);

    let xml = introspect(&cfg, &registry, "/package/nonexistent").unwrap();
    assert!(xml.contains("<node"), "Boş node döndürmeli");
}

// ── Locale yönetimi ──────────────────────────────────────────────────────────

#[tokio::test]
async fn locale_default_is_en() {
    let locales = new_locale_map();
    assert_eq!(sender_language(&locales, ":1.99").await, "en");
}

#[tokio::test]
async fn locale_set_and_retrieve() {
    let locales = new_locale_map();
    locales.write().await.insert(":1.42".into(), "tr".into());
    locales.write().await.insert(":1.43".into(), "de".into());

    assert_eq!(sender_language(&locales, ":1.42").await, "tr");
    assert_eq!(sender_language(&locales, ":1.43").await, "de");
    assert_eq!(sender_language(&locales, ":1.99").await, "en"); // varsayılan
}

#[tokio::test]
async fn locale_overwrite() {
    let locales = new_locale_map();
    locales.write().await.insert(":1.42".into(), "tr".into());
    locales.write().await.insert(":1.42".into(), "fr".into()); // üzerine yaz

    assert_eq!(sender_language(&locales, ":1.42").await, "fr");
}
