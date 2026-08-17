# ÇOMAR Değişiklik Günlüğü

## 3.1.0 — Rust Rewrite (devam ediyor)

### Temel Değişiklikler

**Dil ve derleme sistemi**
- C + CMake → Rust + Cargo
- `comar` binary release olarak derlenir: `cargo build --release`
- Python betikleri aynen korunur (geriye dönük uyumluluk)

**D-Bus katmanı**
- `libdbus-1` (C) → `zbus` (saf Rust, async/tokio)
- `poll()` + `fork()` döngüsü → `tokio::select!` + `spawn_blocking`
- Her D-Bus isteği fork yerine blocking thread pool'da çalışır
- PolicyKit: `libpolkit-gobject-1` yerine doğrudan D-Bus çağrısı (`zbus`)

**Python entegrasyonu**
- Python embed: manuel Python.h API'si → `pyo3` crate
- CSL built-in'leri `__builtin__` yerine `builtins` modülüne eklenir (Python 3)
- Thread-local `RequestContext` ile bağlam bilgisi iletilir (fork yokluğu nedeniyle)
- `notify()` / `call()` → `mpsc` kanal köprüsü ile async olay döngüsüne iletilir

**XML model yükleme**
- Gömülü iksemel XML parser → `quick-xml` crate
- Tüm model dosyaları (`models/*.xml`) değişmez

**Workspace yapısı**
```
comar-daemon/   ← Sistem servisi (eski C daemon)
comar-api/      ← İstemci kütüphanesi (eski Python comar paketi)
comar-py/       ← Python binding (PyO3/maturin, eski comar-api Python paketi)
```

### Korunan Davranışlar

- D-Bus servis adı: `tr.org.pardus.comar`
- D-Bus nesne yol şeması: `/`, `/package`, `/package/<app>`
- D-Bus arayüz şeması: `tr.org.pardus.comar`, `tr.org.pardus.comar.<Model>`
- Dosya sistemi düzeni: `/var/db/comar3/{models,modules,scripts,apps}`
- D-Bus aktivasyon modeli (sürekli servis değil)
- Idle timeout (varsayılan: 60s)
- PolicyKit `.policy` dosyaları (17 adet, değişmez)
- Python model betikleri (geriye dönük uyumluluk korunur)

### Kırılan Uyumluluklar

**Python 2 → Python 3**

Mevcut Python model betikleri Python 2 ile yazılmıştır. `script.rs`'deki
`compat_python3()` fonksiyonu otomatik dönüşüm yapar, ancak karmaşık betikler
için manuel güncelleme gerekebilir:

| Python 2 | Python 3 |
|----------|----------|
| `print x` | `print(x)` |
| `basestring` | `str` |
| `d.iteritems()` | `d.items()` |
| `d.itervalues()` | `d.values()` |
| `file(path)` | `open(path)` |
| `has_key(k)` | `k in d` |
| `except E, e:` | `except E as e:` |

**`config_modelbase()` built-in**
- Python 2'de PyDict, Python 3'te `dict` döndürür
- Model betikleri bu fonksiyonu kullanıyorsa `dict` API'si kullanılmalı

**Sinyal ve iç çağrı gecikme modeli**
- C'de `notify()` / `call()` senkron çalışırdı
- Rust'ta bu çağrılar `mpsc` kanalı üzerinden async event döngüsüne iletilir
- Sonuç: sinyal gönderimi ve iç çağrılar mikrosaniye gecikmeli olabilir
- Pratikte fark edilmez; betikler için davranış aynıdır

### Geliştirici Notları

**Yeni bağımlılıklar**
```toml
zbus        = "4"      # D-Bus istemci/sunucu
pyo3        = "0.21"   # Python embed
quick-xml   = "0.36"   # XML parser
tokio       = "1"      # Async runtime
```

**Test koşturma**
```bash
cargo test --workspace           # Tüm testler
cargo test -p comar-daemon       # Yalnızca daemon testleri
cargo test db_tests              # Model yükleme entegrasyon testleri
```

## 3.0.3 — Son C Sürümü

- Orijinal C + Python implementasyonu
- Kaynak: https://github.com/PisiLinuxNew/comar
