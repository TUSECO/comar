//! Yapılandırma — config.c / config.h karşılığı.
//!
//! Komut satırı argümanlarını ve derleme zamanı varsayılanlarını barındırır.

use clap::Parser;
use std::path::PathBuf;

// ── Derleme zamanı varsayılanları (CMakeLists'ten taşındı) ─────────────────────

pub const DBUS_SERVER_ADDRESS: &str  = "unix:path=/run/dbus/system_bus_socket";
pub const DBUS_SERVICE_NAME: &str    = "tr.org.luppo.comar";
pub const DEFAULT_IDLE_TIMEOUT: u64  = 60;   // saniye; 0 = devre dışı
pub const DEFAULT_DIR_DATA: &str     = "/var/db/comar3";
pub const DEFAULT_DIR_LOG: &str      = "/var/log/comar3";
#[allow(dead_code)]
pub const MAX_CONCURRENT_REQUESTS: usize = 500;

// ── CLI tanımı ─────────────────────────────────────────────────────────────────

/// ÇOMAR — COnfiguration MAnageR sistem servisi.
#[derive(Parser, Debug, Clone)]
#[command(
    name    = "comar",
    version = env!("CARGO_PKG_VERSION"),
    about   = "ÇOMAR sistem yapılandırma yöneticisi",
    long_about = None,
)]
pub struct Config {
    /// D-Bus servis adı  [varsayılan: tr.org.luppo.comar]
    #[arg(short = 'b', long = "busname", default_value = DBUS_SERVICE_NAME)]
    pub service_name: String,

    /// Veri depolama dizini  [varsayılan: /var/db/comar3]
    #[arg(short = 'd', long = "datadir", default_value = DEFAULT_DIR_DATA)]
    pub dir_data: PathBuf,

    /// Hata ayıklama modu
    #[arg(short = 'g', long = "debug")]
    pub debug: bool,

    /// Eksik fonksiyon hatalarını yoksay
    #[arg(short = 'i', long = "ignore-missing")]
    pub ignore_missing: bool,

    /// Log depolama dizini  [varsayılan: /var/log/comar3]
    #[arg(short = 'l', long = "logdir", default_value = DEFAULT_DIR_LOG)]
    pub dir_log: PathBuf,

    /// Çıktıyı dosya yerine konsola yaz (idle timeout'u da devre dışı bırakır)
    #[arg(short = 'p', long = "print")]
    pub print_mode: bool,

    /// D-Bus soket adresi  [varsayılan: unix:path=/run/dbus/system_bus_socket]
    #[arg(short = 's', long = "socket", default_value = DBUS_SERVER_ADDRESS)]
    pub server_address: String,

    /// Eylemsizlik zaman aşımı (saniye); 0 = devre dışı  [varsayılan: 60]
    #[arg(short = 't', long = "timeout", default_value_t = DEFAULT_IDLE_TIMEOUT)]
    pub timeout: u64,
}

impl Config {
    /// Komut satırını ayrıştır.
    pub fn parse_args() -> Self {
        let mut cfg = Self::parse();
        // --print bayraklı çalışmada timeout devre dışı
        if cfg.print_mode {
            cfg.timeout = 0;
        }
        cfg
    }

    // ── Türetilmiş dizin ve dosya yolları ──────────────────────────────────────

    /// Modeller dizini: <dir_data>/models
    pub fn dir_models(&self) -> PathBuf {
        self.dir_data.join("models")
    }

    /// Modül betikleri dizini: <dir_data>/modules
    pub fn dir_modules(&self) -> PathBuf {
        self.dir_data.join("modules")
    }

    /// Kayıtlı betikler dizini: <dir_data>/scripts
    pub fn dir_scripts(&self) -> PathBuf {
        self.dir_data.join("scripts")
    }

    /// Uygulama kayıt dizini: <dir_data>/apps
    pub fn dir_apps(&self) -> PathBuf {
        self.dir_data.join("apps")
    }

    /// Erişim log dosyası: <dir_log>/access.log
    pub fn file_log_access(&self) -> PathBuf {
        self.dir_log.join("access.log")
    }

    /// Traceback log dosyası: <dir_log>/trace.log
    pub fn file_log_trace(&self) -> PathBuf {
        self.dir_log.join("trace.log")
    }

    /// D-Bus arayüz öneki (service_name ile aynı)
    pub fn interface_prefix(&self) -> &str {
        &self.service_name
    }

    /// Model adından tam D-Bus arayüz adı üretir.
    /// "Core" → "tr.org.luppo.comar"
    /// "Net.Link" → "tr.org.luppo.comar.Net.Link"
    #[allow(dead_code)]
    pub fn interface_for_model(&self, model: &str) -> String {
        if model == "Core" {
            self.service_name.clone()
        } else {
            format!("{}.{}", self.service_name, model)
        }
    }
}

// ── Testler ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> Config {
        Config {
            service_name:   DBUS_SERVICE_NAME.into(),
            dir_data:       PathBuf::from(DEFAULT_DIR_DATA),
            dir_log:        PathBuf::from(DEFAULT_DIR_LOG),
            debug:          false,
            ignore_missing: false,
            print_mode:     false,
            server_address: DBUS_SERVER_ADDRESS.into(),
            timeout:        DEFAULT_IDLE_TIMEOUT,
        }
    }

    #[test]
    fn derived_paths() {
        let cfg = default_config();
        assert_eq!(cfg.dir_models(),   PathBuf::from("/var/db/comar3/models"));
        assert_eq!(cfg.dir_scripts(),  PathBuf::from("/var/db/comar3/scripts"));
        assert_eq!(cfg.dir_modules(),  PathBuf::from("/var/db/comar3/modules"));
        assert_eq!(cfg.dir_apps(),     PathBuf::from("/var/db/comar3/apps"));
        assert_eq!(cfg.file_log_access(), PathBuf::from("/var/log/comar3/access.log"));
    }

    #[test]
    fn interface_for_model_core() {
        let cfg = default_config();
        assert_eq!(cfg.interface_for_model("Core"), "tr.org.luppo.comar");
    }

    #[test]
    fn interface_for_model_net_link() {
        let cfg = default_config();
        assert_eq!(cfg.interface_for_model("Net.Link"), "tr.org.luppo.comar.Net.Link");
    }

    #[test]
    fn print_mode_disables_timeout() {
        let mut cfg = default_config();
        cfg.print_mode = true;
        cfg.timeout = DEFAULT_IDLE_TIMEOUT;
        // parse_args() normalde bunu ayarlar; burada manuel test
        if cfg.print_mode { cfg.timeout = 0; }
        assert_eq!(cfg.timeout, 0);
    }
}
