// build.rs — comar-py PyO3 extension module derleme yapılandırması.
//
// pyo3-build-config:
//   * Python sürümüne özgü linker bayraklarını ekler
//     (örn. macOS'ta `-undefined dynamic_lookup`)
//   * Doğru Python ABI suffix'ini (`.cpython-312-x86_64-linux-gnu.so`)
//     belirler; bu Cargo'ya değil kurulum adımına bilgi verir.
//
// NOT: Linux'ta cdylib çıktısı her zaman `lib` öneki alır (libcomar.so).
// Python bu dosyayı `comar.so` olarak arar. Yeniden adlandırma işlemi
// Makefile'daki `install-py` hedefinde veya maturin aracıyla yapılır.
// Bu build.rs yalnızca linker uyumluluğunu sağlar.

fn main() {
    pyo3_build_config::add_extension_module_link_args();
}
