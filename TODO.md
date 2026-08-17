# ÇOMAR Rust Eksikler ve TODO Listesi

Bu liste eski `comar` ve `comar-api` projeleri ile yeni `comar-rust` projesi arasındaki farklara ve eksikliklere göre oluşturulmuştur.

## 1. Python API Bağlamaları (`comar-py`)
- [x] `comar-py/src/lib.rs` içerisindeki `PyLink` metodlarını (`setLocale`, `listRunning`, `cancel`, `register`, `remove`) sadece `Ok(())` dönen mock fonksiyonlar olmaktan çıkarıp, gerçek D-Bus çağrılarına dönüştür.
- [x] `PyCall` sınıfının `__call__` metodunu, argümanları ayrıştırıp gerçek bir D-Bus metodu çağıracak şekilde uygula.
- [x] `listenSignals` metodu için Python tarafında sinyal dinleme (async/tokio + zbus) entegrasyonunu tamamla.

## 2. Eksik Python Yardımcı Modülleri
Eski `comar-api` içerisindeki Python betikleri için gerekli olan yardımcı kütüphanelerin yeni projeye (`comar-py/comar/` dizinine) aktarılması/uyarlanması:
- [x] `network.py` modülünün taşınması.
- [x] `service.py` modülünün taşınması.
- [x] `utility.py` modülünün taşınması.

## 3. Eski TODO Dosyasındaki (Daemon) Eksiklikler
- [x] Loglama mekanizmasında hata (error) ve Python traceback loglarını ayrı bir dosyaya yazma desteği.
- [x] Model içe aktarımı ve tip dönüşümlerinde (Numeric types, Variant type) oluşan hataların düzgün fırlatılması (Exception on conversion error).
- [x] Eski COMAR 2.0 policy action ID'lerini güncel PolicyKit (polkit) yapılarına uyarlama kontrolleri.

- [x] i18n desteği (rust-i18n entegrasyonu tamamlandı)
- [x] Python bağımsızlığı (Deklaratif .toml ve .json servis desteği eklendi)

## 4. Eski Proje İşlevleri ve Sorumlulukları

Eski `comar` ve `comar-api` projeleri, sistem yapılandırmasını otomatize etmek ve merkezi bir yönetim sağlamak amacıyla geliştirilmiştir. Aşağıda bu projelerin temel sorumlulukları listelenmiştir:

### ÇOMAR Daemon (`comar`)
*   **Merkezi Yapılandırma Yönetimi**: Donanım, ağ, kullanıcı, zaman ve görüntü ayarlarının merkezi bir noktadan (D-Bus üzerinden) yönetilmesini sağlar.
*   **Betik Motoru**: Python tabanlı yapılandırma betiklerini (`scripts/`) çalıştırır. Her uygulama kendi "model"ine uygun bir Python betiği sağlar.
*   **Model ve Kayıt Sistemi**: XML tabanlı model dosyaları (`models/`) ile arayüzleri tanımlar. Hangi uygulamanın hangi modeli desteklediğini takip eder.
*   **Yetkilendirme (PolicyKit)**: Hassas sistem ayarları için PolicyKit üzerinden kullanıcı yetki denetimi yapar.
*   **Olay Dinleme**: Sistemdeki değişiklikleri (donanım takılması, ağ durumu vb.) izleyerek ilgili betikleri tetikler.

### ÇOMAR API (`comar-api`)
*   **Client Kütüphanesi**: Uygulamaların ÇOMAR daemon ile D-Bus üzerinden kolayca haberleşmesini sağlayan Python kütüphanesidir.
*   **Dinamik Erişim**: `comar.Link()` üzerinden dinamik metod çağrıları (örn: `Link.System.Service["apache"].start()`) yapılmasına olanak tanır.
*   **Yardımcı Modüller**:
    *   `network.py`: Ağ ayarları için yardımcı fonksiyonlar.
    *   `service.py`: Servis yönetimi (start/stop/status) kolaylaştırıcıları.
    *   `utility.py`: Ortak kullanılan yardımcı araçlar.

### Temel Uygulama Alanları (Modeller)
COMAR aşağıdaki ana alanlarda yapılandırma sunar:
*   **Sistem**: Paket yönetimi (Luppo), servis yönetimi, genel sistem ayarları.
*   **Ağ**: Kablolu/kablosuz bağlantılar, firewall yönetimi.
*   **Kullanıcı**: Kullanıcı ekleme/çıkarma, yetki tanımları.
*   **Donanım**: Disk yönetimi, ekran kartı (Xorg) ayarları.
*   **Zaman/Tarih**: Saat ve bölge ayarları.
*   **Önyükleyici (Boot)**: Bootloader ve kernel modülleri yönetimi.


## 5. ÇOMAR Rust Projesi İşlevleri

`comar-rust`, eski C tabanlı yapının modern ve güvenli Rust diliyle yeniden yazılmış halidir. Sistemin temel işlevlerini korurken modern altyapı avantajlarını sunar:

### Modernize Edilmiş Daemon (`comar-daemon`)
*   **Bellek Güvenliği**: Rust'ın sahiplik (ownership) modeli sayesinde C'deki bellek sızıntıları ve "buffer overflow" riskleri ortadan kaldırılmıştır.
*   **Asenkron Mimari**: `tokio` tabanlı asenkron çalışma sayesinde birden fazla D-Bus isteği ve sinyal işlemi eşzamanlı olarak yönetilebilir.
*   **Gelişmiş Loglama ve Traceback**: Hataların tespiti için `tracing` kütüphanesi kullanılır. Python betik hatalarında tam "traceback" desteği sunarak `/var/log/comar3/comar-error.log` dosyasına detaylı raporlama yapar.
*   **Tip Güvenliği**: D-Bus üzerindeki veriler `zvariant` ile Rust tiplerine güvenli bir şekilde eşlenir, tip uyuşmazlıkları çalışma anında hata olarak yakalanır.
*   **Python 3 ve Geriye Uyumluluk**: Mevcut Python 2 betiklerini Python 3 ortamında çalıştırabilmek için otomatik uyumluluk yamaları (`compat_python3`) uygular.

### Rust Tabanlı Python Bindings (`comar-py`)
*   **Yüksek Performanslı Köprü**: `pyo3` kütüphanesi ile Rust ve Python arasında düşük gecikmeli veri transferi sağlar.
*   **Tam API Uyumluluğu**: Eski kütüphane ile aynı metod isimlerini ve parametre yapısını koruyarak mevcut uygulamaların (örn: LupuS Yazılım Merkezi) değişikliğe gerek duymadan çalışmasını sağlar.

## 6. Proje Karşılaştırması (Eski vs. Yeni)

| Özellik | Eski COMAR (C/Python) | Yeni COMAR-Rust (Rust) |
| :--- | :--- | :--- |
| **Dil** | C (Daemon) + Python (API) | Rust (Daemon & Core API) |
| **D-Bus Kütüphanesi** | libdbus / pydbus | zbus (Yerel Rust kütüphanesi) |
| **Çalışma Modu** | Senkron / Tekil İşlem | Asenkron / Çoklu Görev (Tokio) |
| **Bellek Yönetimi** | Manuel (C) - Sızıntı riski | Otomatik & Güvenli (Rust) |
| **Python Desteği** | Python 2 (Eski standart) | Python 3 (Modern standart) |
| **Hata Analizi** | Kısıtlı loglama | Detaylı Traceback + Ayrı Hata Logu |
| **Genişletilebilirlik** | C makroları ve karmaşık yapı | Modern Cargo paket yönetimi ve modülerlik |
| **Güvenlik** | Geleneksel PolicyKit | Modern Polkit + Güvenli Tip Dönüşümü |
| **Uyumluluk** | - | Eski XML modelleri ve betikleriyle %100 uyumlu |
