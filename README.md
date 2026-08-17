# ÇOMAR — LupuS D-Bus IPC Altyapısı

COMAR (COnfiguration MAnageR), LupuS GNU/Linux'un servis/paket yönetimi için kullandığı
D-Bus IPC altyapısının Rust ile yeniden yazımıdır. Python betiklerini KDL bildirim diline
dönüştürerek daha güvenli, hızlı ve bakımı kolay bir sistem sunmayı hedefler.

## Mimari

```
┌──────────────────────────────────────────────────┐
│  comar-daemon  (D-Bus system bus, tokio async)   │
│                                                  │
│  ┌─────────────┐  ┌────────────┐  ┌──────────┐   │
│  │ Declarative │  │   Native   │  │  Python  │   │
│  │   (KDL)     │  │  Services  │  │  Legacy  │   │
│  └─────────────┘  └────────────┘  └──────────┘   │
│       │                │              │          │
│       ▼                ▼              ▼          │
│  service.kdl     User.Manager      service.py    │
│  package.kdl     Network.Stack     package.py    │
│  handlers/*.kdl  System.PackageHandler           │
└──────────────────────────────────────────────────┘
```

**Üç katmanlı dispatch:**
1. **Native Rust servisleri** — kritik sistem servisleri (`User.Manager`,
   `Network.Stack`, `System.PackageHandler`) doğrudan Rust'da
2. **KDL bildirim servisleri** — `service.kdl`/`package.kdl` dosyaları shell
   komutları olarak çalıştırılır
3. **Python legacy** — dönüştürülmemiş `service.py`/`package.py` PyO3 ile
   çalıştırılmaya devam eder

## Proje Yapısı

```
├── Cargo.toml                     # Workspace root
├── README.md
├── locales/                       # i18n çevirileri
├── comar-daemon/                  # D-Bus arkaplan süreci
│   ├── src/
│   │   ├── main.rs                # Giriş noktası
│   │   ├── lib.rs                 # Kütüphane arayüzü
│   │   ├── config.rs              # Yapılandırma
│   │   ├── declarative.rs         # KDL çalıştırıcı
│   │   ├── process.rs             # D-Bus olay döngüsü
│   │   ├── script.rs              # Python VM yönetimi
│   │   ├── services/
│   │   │   ├── mod.rs             # Service dispatch
│   │   │   ├── user_manager.rs    # User.Manager
│   │   │   ├── network_stack.rs   # Network.Stack
│   │   │   └── package_handler.rs # System.PackageHandler
│   │   ├── bus.rs                 # Mesaj yönlendirme
│   │   ├── core.rs                # Core D-Bus metodları
│   │   ├── db.rs                  # Model veritabanı
│   │   ├── error.rs               # Hata tipleri
│   │   ├── policy.rs              # PolicyKit
│   │   └── signal_bus.rs          # Sinyal yönetimi
│   └── Cargo.toml
├── comar-api/                     # Rust API kütüphanesi
├── comar-py/                      # Python API (güncellenecek)
├── tools/
│   └── comar2kdl.py               # Python → KDL dönüştürücü
└── examples/
    ├── service.kdl                # System.Service şablonu
    ├── package.kdl                # System.Package şablonu
    ├── package-dbus.kdl           # dbus örneği
    ├── package-glibc.kdl          # glibc örneği
    ├── package-pam.kdl            # PAM örneği
    └── handlers/                  # System.PackageHandler örnekleri
        ├── baselayout.kdl         # Baselayout dosya yönetimi
        ├── fontconfig.kdl         # Font önbellek yenileme
        ├── kmod.kdl               # Kernel modül kurulumu
        ├── python.kdl             # Python bytecode derleme
        ├── dkms.kdl               # DKMS modül yönetimi
        ├── desktop-file-utils.kdl # MIME tipi güncelleme
        └── ... (22 adet)
```

## Derleme

```bash
# Tüm workspace'i derle
cargo build --release

# Sadece daemon
cargo build -p comar-daemon

# Test
cargo test -p comar-daemon
```

## Kullanım

```bash
# Kök yetkileri gerekli
sudo ./target/release/comar-daemon

# Hata ayıklama modu
sudo ./target/release/comar-daemon --debug

# Özel dizinler
sudo ./target/release/comar-daemon \
  --dir-modules /var/db/comar3/modules \
  --dir-models /var/db/comar3/models \
  --dir-scripts /var/db/comar3/scripts
```

## KDL Dönüşümü

COMAR Python betiklerini KDL'ye dönüştürmek için:

```bash
# Tek dosya
python3 tools/comar2kdl.py --file service.py

# Tüm paketler (toplu)
python3 tools/comar2kdl.py --recursive --dir /path/to/paketler/

# Sadece özet
python3 tools/comar2kdl.py --recursive --dir /path/to/ --summary
```

## Servis Dispatch

`services/mod.rs`'e yeni bir model eklendiğinde, o modelin tüm çağrıları
Python yerine doğrudan Rust'da işlenir:

```rust
pub fn dispatch(model: &str, method: &str, args: &[OwnedValue], lang: &str) -> ServiceDispatch {
    match model {
        "Network.Stack" => Native(NetworkStack.handle(method, args, lang)),
        "User.Manager"  => Native(UserManager.handle(method, args, lang)),
        "System.PackageHandler" => Native(package_handler::run_all(method, args)),
        _ => Passthrough,  // Python / KDL
    }
}
```

## Lisans

GPLv3 — LupuS GNU/Linux
