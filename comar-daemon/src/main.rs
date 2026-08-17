//! ÇOMAR daemon — giriş noktası.
//!
//! main.c'deki akışı birebir karşılar:
//!   config_init → log_init → script_init → proc_init → loop_exec → script_finalize

mod bus;

rust_i18n::i18n!("../../locales", fallback = "tr");
use rust_i18n::t;
mod config;
mod core;
mod db;
mod error;
mod logging;
mod policy;
mod process;
mod script;
mod services;
mod declarative;
mod signal_bus;

use crate::config::Config;
use crate::error::Result;
use std::sync::Arc;
use tracing::{error, info};

#[tokio::main]
async fn main() {
    let cfg = Config::parse_args();

    // Root kontrolü (main.c: if (getuid() != 0))
    if !nix::unistd::getuid().is_root() {
        eprintln!("{}", t!("error_root_required"));
        std::process::exit(1);
    }

    // Loglama başlat
    let log_file = if cfg.print_mode {
        None
    } else {
        Some(cfg.file_log_access())
    };

    if let Err(e) = logging::init(cfg.print_mode, cfg.debug, log_file.as_deref()) {
        eprintln!("{}", t!("error_log_init_failed", error = e));
        std::process::exit(1);
    }

    info!("{}", t!("status_starting", version = env!("CARGO_PKG_VERSION")));
    info!("Modüller dizini  : {:?}", cfg.dir_modules());
    info!("Modeller dizini  : {:?}", cfg.dir_models());
    info!("Betikler dizini  : {:?}", cfg.dir_scripts());
    info!("Erişim logu      : {:?}", cfg.file_log_access());
    info!("Trace logu       : {:?}", cfg.file_log_trace());

    // Gerekli dizinleri kontrol et ve oluştur
    if let Err(e) = ensure_dirs(&cfg) {
        error!("{}", t!("error_dir_generic", error = e));
        std::process::exit(1);
    }

    // XML modelleri yükle — db_load_models() karşılığı
    let registry = match db::load_models(&cfg.dir_models(), cfg.interface_prefix()) {
        Ok(r) => {
            info!("{}", t!("status_models_loaded", count = r.len()));
            Arc::new(r)
        }
        Err(e) => {
            error!("{}", t!("error_model_load_failed", error = e));
            std::process::exit(1);
        }
    };

    // Python VM başlat — script_init() karşılığı
    if let Err(e) = script::init(&cfg, Arc::clone(&registry)) {
        error!("{}", t!("error_python_init_failed", error = e));
        std::process::exit(1);
    }
    info!("{}", t!("status_python_ready"));

    // Ana olay döngüsünü başlat — loop_exec() karşılığı
    if let Err(e) = process::run(Arc::new(cfg), Arc::clone(&registry)).await {
        error!("{}", t!("error_daemon_failed", error = e));
        std::process::exit(1);
    }

    // Python VM kapat — script_finalize() karşılığı
    script::finalize();

    info!("{}", t!("status_shutdown"));
}

/// Gerekli dizinleri doğrula ve yoksa oluştur.
///
/// main.c: check_dir / create_dir karşılığı.
fn ensure_dirs(cfg: &Config) -> Result<()> {
    use std::fs;

    // Var olması gereken dizinler
    for dir in [cfg.dir_data.as_path(), &cfg.dir_models(), &cfg.dir_modules()] {
        if !dir.exists() {
            return Err(crate::error::ComarError::Missing(t!(
                "error_dir_not_found",
                dir = dir.to_string_lossy()
            ).to_string()));
        }
    }

    // Oluşturulacak dizinler
    for dir in [
        cfg.dir_scripts(),
        cfg.dir_apps(),
        cfg.dir_log.clone(),
    ] {
        fs::create_dir_all(&dir).map_err(crate::error::ComarError::Io)?;
    }

    Ok(())
}
