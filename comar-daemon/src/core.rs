//! Core arayüzü — modules/core.py + handle_core_message() karşılığı.
//!
//! tr.org.luppo.comar arayüzündeki temel sistem metodları:
//! register, remove, listModelApplications, listApplicationModels,
//! introspect, setLocale, cancel, listRunning

use crate::config::Config;
use crate::db::{ModelDef, ModelRegistry, MemberType};
use crate::error::{ComarError, Result};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

// ── Locale deposu ─────────────────────────────────────────────────────────────

/// Gönderici D-Bus adı → dil kodu eşlemesi.
///
/// C'deki `py_core["locales"]` dict'inin karşılığı.
/// `setLocale` / `sender_language()` için kullanılır.
pub type LocaleMap = Arc<RwLock<HashMap<String, String>>>;

/// Boş `LocaleMap` oluşturur.
pub fn new_locale_map() -> LocaleMap {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Göndericinin dilini döndürür; kayıtlı değilse `"en"`.
///
/// `sender_language()` karşılığı.
pub async fn sender_language(locales: &LocaleMap, sender: &str) -> String {
    locales
        .read()
        .await
        .get(sender)
        .cloned()
        .unwrap_or_else(|| "en".to_string())
}

// ── Aktif istek kaydı ─────────────────────────────────────────────────────────

/// Aktif bir istek kaydı.
///
/// C'deki `struct ProcChild` karşılığı; fork() yerine task handle.
#[derive(Debug)]
pub struct ActiveRequest {
    pub sender:    String,
    pub interface: String,
    pub method:    String,
    pub handle:    tokio::task::JoinHandle<()>,
}

pub type ActiveRequests = Arc<RwLock<Vec<ActiveRequest>>>;

pub fn new_active_requests() -> ActiveRequests {
    Arc::new(RwLock::new(Vec::new()))
}

// ── Paket kayıt & silme ───────────────────────────────────────────────────────

/// Bir paketi bir model ile ilişkilendirir ve betiği kopyalar.
///
/// `core.py register()` karşılığı.
///
/// Dosya sistemi yapısı:
/// - `<scripts_dir>/<model>/<app>.py`  ← betik kopyası
/// - `<apps_dir>/<app>/<model>`        ← boş işaret dosyası
pub fn register(
    cfg: &Config,
    app: &str,
    model: &str,
    script_path: &str,
) -> Result<()> {
    let script_src = Path::new(script_path);
    let ext = script_src.extension().and_then(|s| s.to_str()).unwrap_or("py");

    // scripts/<model>/<app>.<ext>
    let script_dir = cfg.dir_scripts().join(model);
    fs::create_dir_all(&script_dir).map_err(ComarError::Io)?;
    let script_dst = script_dir.join(format!("{}.{}", app, ext));
    fs::copy(script_src, &script_dst).map_err(ComarError::Io)?;

    // apps/<app>/<model>  (boş işaret dosyası)
    let app_dir = cfg.dir_apps().join(app);
    fs::create_dir_all(&app_dir).map_err(ComarError::Io)?;
    let marker = app_dir.join(model);
    fs::write(&marker, b"").map_err(ComarError::Io)?;

    info!("Kayıt edildi: app={} model={}", app, model);
    Ok(())
}

/// Bir paketin tüm kayıtlarını ve betiklerini siler.
///
/// `core.py remove()` karşılığı.
pub fn remove(cfg: &Config, app: &str) -> Result<()> {
    let app_dir = cfg.dir_apps().join(app);
    if !app_dir.exists() {
        return Ok(()); // Zaten yok
    }

    // apps/<app>/ içindeki her model için scripts/<model>/<app>.* dosyasını sil
    for entry in fs::read_dir(&app_dir).map_err(ComarError::Io)?.flatten() {
        let model_name = entry.file_name();
        let model_name = model_name.to_string_lossy();
        if model_name.starts_with('.') {
            continue;
        }
        let extensions = ["kdl", "py", "json"];
        for ext in extensions.iter() {
            let script = cfg
                .dir_scripts()
                .join(model_name.as_ref())
                .join(format!("{}.{}", app, ext));
            if script.exists() {
                fs::remove_file(&script).map_err(ComarError::Io)?;
                debug!("Silindi: {:?}", script);
            }
        }
    }

    // apps/<app>/ dizinini sil
    fs::remove_dir_all(&app_dir).map_err(ComarError::Io)?;
    info!("Kaldırıldı: app={}", app);
    Ok(())
}

// ── Listeleme ─────────────────────────────────────────────────────────────────

/// Belirli bir modeli uygulayan paketleri listeler.
///
/// `core.py listModelApplications()` karşılığı.
/// `scripts/<model>/` dizinindeki `.py` dosyalarını tarar.
pub fn list_model_applications(cfg: &Config, model: &str) -> Result<Vec<String>> {
    let dir = cfg.dir_scripts().join(model);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut apps: Vec<String> = fs::read_dir(&dir)
        .map_err(ComarError::Io)?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let path = std::path::Path::new(name.as_ref());
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy();
                if ext_str == "kdl" || ext_str == "py" || ext_str == "json" {
                    return path.file_stem().map(|s| s.to_string_lossy().into_owned());
                }
            }
            None
        })
        .collect();
    apps.sort();
    apps.dedup();
    Ok(apps)
}

/// Bir paketin uyguladığı modelleri listeler.
///
/// `core.py listApplicationModels()` karşılığı.
/// `apps/<app>/` dizinindeki girişleri tarar.
pub fn list_application_models(cfg: &Config, app: &str) -> Result<Vec<String>> {
    let dir = cfg.dir_apps().join(app);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let models = fs::read_dir(&dir)
        .map_err(ComarError::Io)?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with('.') {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect();
    Ok(models)
}

// ── Introspection XML üretimi ─────────────────────────────────────────────────

/// D-Bus Introspection XML'i oluşturur.
///
/// `core.py introspect()` + `model_xml()` karşılığı.
/// `path` değerine göre farklı içerik üretir:
///
/// - `/`              → Core arayüzü + `<node name="package"/>`
/// - `/package`       → kayıtlı uygulama düğümleri
/// - `/package/<app>` → o uygulamanın model arayüzleri
pub fn introspect(cfg: &Config, registry: &ModelRegistry, path: &str) -> Result<String> {
    let mut xml = String::from(
        "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\"\n\
         \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n"
    );
    xml.push_str(&format!("<node name=\"{}\">\n", path));

    if path == "/" {
        // Core arayüzü
        xml.push_str(&model_interface_xml(
            &cfg.service_name,
            registry.get("Core"),
            &cfg.service_name,
        ));
        // Introspectable arayüzü (standart)
        xml.push_str(INTROSPECTABLE_XML);
        xml.push_str("  <node name=\"package\"/>\n");
    } else if path == "/package" {
        // Tüm kayıtlı uygulamalar
        let apps_dir = cfg.dir_apps();
        if apps_dir.exists() {
            for entry in fs::read_dir(&apps_dir).map_err(ComarError::Io)?.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.starts_with('.') {
                    xml.push_str(&format!("  <node name=\"{}\"/>\n", name));
                }
            }
        }
    } else if let Some(app) = path.strip_prefix("/package/") {
        // Uygulamanın model arayüzleri
        let models = list_application_models(cfg, app)?;
        for model_name in &models {
            let iface_name = if model_name == "Core" {
                cfg.service_name.clone()
            } else {
                format!("{}.{}", cfg.service_name, model_name)
            };
            xml.push_str(&model_interface_xml(
                &iface_name,
                registry.get(model_name.as_str()),
                &cfg.service_name,
            ));
        }
        xml.push_str(INTROSPECTABLE_XML);
    }

    xml.push_str("</node>\n");
    Ok(xml)
}

/// Tek bir model için D-Bus `<interface>` XML bloğu üretir.
///
/// `core.py model_xml()` karşılığı.
fn model_interface_xml(
    iface_name: &str,
    model_def: Option<&ModelDef>,
    _prefix: &str,
) -> String {
    let mut xml = format!("  <interface name=\"{}\">\n", iface_name);

    if let Some(members) = model_def {
        for (name, def) in members {
            let tag = if def.kind == MemberType::Method {
                "method"
            } else {
                "signal"
            };
            xml.push_str(&format!("    <{} name=\"{}\">\n", tag, name));
            for t in &def.args_in {
                xml.push_str(&format!(
                    "      <arg type=\"{}\" direction=\"in\"/>\n",
                    t
                ));
            }
            for t in &def.args_out {
                xml.push_str(&format!(
                    "      <arg type=\"{}\" direction=\"out\"/>\n",
                    t
                ));
            }
            xml.push_str(&format!("    </{}>\n", tag));
        }
    }

    xml.push_str("  </interface>\n");
    xml
}

const INTROSPECTABLE_XML: &str =
    "  <interface name=\"org.freedesktop.DBus.Introspectable\">\n\
         <method name=\"Introspect\">\n\
           <arg type=\"s\" direction=\"out\"/>\n\
         </method>\n\
       </interface>\n";

// ── Testler ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DBUS_SERVICE_NAME, DBUS_SERVER_ADDRESS, DEFAULT_IDLE_TIMEOUT};

    fn test_config(tmp: &Path) -> Config {
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

    #[test]
    fn register_creates_files() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());

        // Gerekli dizinleri oluştur
        fs::create_dir_all(cfg.dir_scripts()).unwrap();
        fs::create_dir_all(cfg.dir_apps()).unwrap();

        // Sahte betik dosyası
        let script_path = tmp.path().join("test_script.py");
        fs::write(&script_path, b"def start(): pass").unwrap();

        register(&cfg, "apache", "System.Service", script_path.to_str().unwrap()).unwrap();

        // scripts/System.Service/apache.py oluştu mu?
        assert!(cfg.dir_scripts().join("System.Service").join("apache.py").exists());
        // apps/apache/System.Service işaret dosyası oluştu mu?
        assert!(cfg.dir_apps().join("apache").join("System.Service").exists());
    }

    #[test]
    fn remove_cleans_up() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());

        fs::create_dir_all(cfg.dir_scripts()).unwrap();
        fs::create_dir_all(cfg.dir_apps()).unwrap();

        // Önce kaydet
        let script_path = tmp.path().join("s.py");
        fs::write(&script_path, b"def start(): pass").unwrap();
        register(&cfg, "apache", "System.Service", script_path.to_str().unwrap()).unwrap();

        // Sonra sil
        remove(&cfg, "apache").unwrap();

        assert!(!cfg.dir_apps().join("apache").exists());
        assert!(!cfg.dir_scripts().join("System.Service").join("apache.py").exists());
    }

    #[test]
    fn list_model_applications_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        fs::create_dir_all(cfg.dir_scripts()).unwrap();
        let apps = list_model_applications(&cfg, "System.Service").unwrap();
        assert!(apps.is_empty());
    }

    #[test]
    fn list_model_applications_after_register() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        fs::create_dir_all(cfg.dir_scripts()).unwrap();
        fs::create_dir_all(cfg.dir_apps()).unwrap();

        let script = tmp.path().join("s.py");
        fs::write(&script, b"").unwrap();
        register(&cfg, "apache", "System.Service", script.to_str().unwrap()).unwrap();
        register(&cfg, "nginx",  "System.Service", script.to_str().unwrap()).unwrap();

        let mut apps = list_model_applications(&cfg, "System.Service").unwrap();
        apps.sort();
        assert_eq!(apps, vec!["apache", "nginx"]);
    }

    #[test]
    fn list_application_models_after_register() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = test_config(tmp.path());
        fs::create_dir_all(cfg.dir_scripts()).unwrap();
        fs::create_dir_all(cfg.dir_apps()).unwrap();

        let script = tmp.path().join("s.py");
        fs::write(&script, b"").unwrap();
        register(&cfg, "apache", "System.Service", script.to_str().unwrap()).unwrap();
        register(&cfg, "apache", "System.Package", script.to_str().unwrap()).unwrap();

        let mut models = list_application_models(&cfg, "apache").unwrap();
        models.sort();
        assert_eq!(models, vec!["System.Package", "System.Service"]);
    }

    #[tokio::test]
    async fn locale_map_default_en() {
        let locales = new_locale_map();
        let lang = sender_language(&locales, ":1.42").await;
        assert_eq!(lang, "en");
    }

    #[tokio::test]
    async fn locale_map_set_and_get() {
        let locales = new_locale_map();
        locales.write().await.insert(":1.42".into(), "tr".into());
        assert_eq!(sender_language(&locales, ":1.42").await, "tr");
        assert_eq!(sender_language(&locales, ":1.99").await, "en");
    }
}
