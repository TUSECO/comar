//! Loglama — log.c karşılığı.
//!
//! tracing-subscriber ile hem dosyaya hem konsola log yazar.

use std::fs::OpenOptions;
use std::path::Path;
use tracing_subscriber::{
    fmt::{self},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
    Layer,
};

/// Loglama altyapısını başlatır.
///
/// * `print_mode` → true ise yalnızca stdout'a yazar (--print flag'i)
/// * `debug`      → true ise TRACE seviyesine kadar loglar
/// * `log_file`   → log dosyası yolu (print_mode=false ise kullanılır)
pub fn init(
    print_mode: bool,
    debug: bool,
    log_file: Option<&Path>,
) -> anyhow::Result<()> {
    let level = if debug { "trace" } else { "info" };

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    if print_mode || log_file.is_none() {
        // Yalnızca konsola yaz
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_level(true)
            .init();
    } else {
        // Hem konsola hem dosyaya yaz
        let log_path = log_file.unwrap();

        // Log dizinini oluştur
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;

        let error_log_path = log_path.with_file_name("comar-error.log");
        let error_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(error_log_path)?;

        use tracing_subscriber::filter::LevelFilter;

        let file_layer = fmt::layer()
            .with_writer(file)
            .with_target(true)
            .with_level(true)
            .with_ansi(false);

        let error_layer = fmt::layer()
            .with_writer(error_file)
            .with_target(true)
            .with_level(true)
            .with_ansi(false)
            .with_filter(LevelFilter::ERROR);

        let console_layer = fmt::layer()
            .with_target(false)
            .with_level(true);

        tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .with(error_layer)
            .with(console_layer)
            .init();
    }

    Ok(())
}
