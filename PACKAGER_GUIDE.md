# LupuS COMAR Betik Rehberi

Bu rehber, paket geliştiricilerinin COMAR betiklerini (`service.py`,
`package.py`) anlaması ve KDL formatına dönüştürmesi için hazırlanmıştır.

## İçindekiler

1. [COMAR Nedir?](#comar-nedir)
2. [System.Service — Servis Yönetimi](#systemservice--servis-yönetimi)
3. [System.Package — Paket Yönetimi](#systempackage--paket-yönetimi)
4. [System.PackageHandler — Paket İşleyiciler](#systempackagehandler--paket-işleyiciler)
5. [Python → KDL Dönüşümü](#python--kdl-dönüşümü)
6. [Sık Kalıplar](#sık-kalıplar)
7. [Test ve Doğrulama](#test-ve-doğrulama)

---

## COMAR Nedir?

COMAR (Component Manager), LupuS'ta servis ve paket yönetimini
D-Bus üzerinden standartlaştıran sistemdir. Her paket, kurulum/servis
mantığını `comar/` dizini altındaki betiklerde tanımlar:

```
paket-adi/
├── lopec.xml (veya lopec.kdl)
└── comar/
    ├── service.py       # Servis yönetimi (start/stop/status)
    ├── package.py       # Paket kurulum/kaldırma
    ├── postInstall.py   # (nadiren) package.py alternatifi
    └── handlers/        # System.PackageHandler kuralları
        └── *.kdl
```

COMAR-RS, bu betikleri üç şekilde işler:

| Katman | Açıklama | Örnekler |
|--------|----------|----------|
| **Native Rust** | Kritik sistem servisleri, doğrudan Rust | `User.Manager`, `Network.Stack` |
| **KDL** | Shell komutları olarak bildirim | `service.kdl`, `package.kdl` |
| **Python (legacy)** | PyO3 ile Python VM'de çalıştırma | `service.py`, `package.py` |

---

## System.Service — Servis Yönetimi

### KDL Formatı

```kdl
service {
    type "local"                    // "local", "server", "virtual"
    description {
        en "Service Name"
        tr "Servis Adı"
    }
    conf "servisadi"                // /etc/conf.d/servisadi yüklenir
    default "conditional"           // "on", "off", "conditional"

    start "/usr/sbin/servis --arg" donotify=true
    stop "kill $(cat /run/servis.pid 2>/dev/null) 2>/dev/null" donotify=true
    status "kill -0 $(cat /run/servis.pid 2>/dev/null) 2>/dev/null"
}
```

### Alanlar

| Alan | Zorunlu | Açıklama |
|------|---------|----------|
| `type` | Evet | Servis tipi: `local` (istemci), `server` (arkaplan), `virtual` |
| `description` | Önerilir | Çok dilli açıklama |
| `conf` | Hayır | `/etc/conf.d/<isim>` dosyasını shell'de `.` ile yükler |
| `default` | Hayır | Varsayılan durum: `on`, `off`, `conditional` |

### Metodlar

Her metod için KDL düğümü yazılır. Metod adı herhangi bir şey olabilir.
`donotify=true` eklendiğinde `start`/`stop` sonrası D-Bus sinyali gönderilir.

**Sık kullanılan metod adları:** `start`, `stop`, `status`, `reload`,
`ready`, `setState`

### Python'daki Karşılıkları

```python
# service.py → KDL
startService(command="/usr/sbin/NetworkManager", args="--opt", donotify=True)
# → start "/usr/sbin/NetworkManager --opt" donotify=true

stopService(pidfile="/run/NetworkManager/NetworkManager.pid", donotify=True)
# → stop "kill $(cat /run/NetworkManager/NetworkManager.pid 2>/dev/null) 2>/dev/null" donotify=true

isServiceRunning("/run/NetworkManager/NetworkManager.pid")
# → status "kill -0 $(cat /run/NetworkManager/NetworkManager.pid 2>/dev/null) 2>/dev/null"
```

### Örnek: DHCPCD

**service.py (orijinal):**
```python
serviceType = "local"
serviceDefault = "conditional"
serviceDesc = _({"en": "Dhcp client", "tr": "Dhcp istemcisi"})
pidfile = "/var/run/dhcpcd.pid"

def start():
    if not USETHIS:
        fail(MSG_BACKEND_WARNING)
    startService(command="/usr/bin/dhcpcd", args="-q ", donotify=True)

def stop():
    stopService(command="/usr/bin/dhcpcd -x", donotify=True)
    try:
        os.unlink(pidfile)
    except:
        pass
```

**service.kdl (dönüştürülmüş):**
```kdl
service {
    type local
    description {
        en "Dhcp client"
        tr "Dhcp istemcisi"
    }
    default conditional
    start "/usr/bin/dhcpcd -q" donotify=true
    stop "kill $(cat /var/run/dhcpcd.pid 2>/dev/null) 2>/dev/null; (rm -f /var/run/dhcpcd.pid) 2>/dev/null || true" donotify=true
}
```

---

## System.Package — Paket Yönetimi

### KDL Formatı

```kdl
package {
    description "Package description"

    post-install "shell commands..."
    pre-remove "shell commands..."
    post-remove "shell commands..."
}
```

### Metodlar

| Metod | Argümanlar | Açıklama |
|-------|-----------|----------|
| `post-install` | `$1=fromVer`, `$2=fromRel`, `$3=toVer`, `$4=toRel` | Paket kurulduktan sonra |
| `pre-remove` | Yok | Paket kaldırılmadan önce |
| `post-remove` | Yok | Paket kaldırıldıktan sonra |

Kısa çizgili adlar otomatik camelCase'e çevrilir:
- `post-install` ↔ `postInstall`
- `pre-remove` ↔ `preRemove`
- `post-remove` ↔ `postRemove`

### Python → KDL Eşlemesi

```python
# Python → KDL
os.system("chmod 4755 /sbin/unix_chkpwd")
# → post-install "chmod 4755 /sbin/unix_chkpwd"

os.chmod("/root", 0o700)
# → chmod 0700 /root

os.unlink("/var/run/something.pid")
# → rm -f /var/run/something.pid

shutil.copy("/etc/passwd", "/usr/share/baselayout/passwd.backup")
# → cp /etc/passwd /usr/share/baselayout/passwd.backup
```

### Örnek: D-Bus

**package.py (orijinal):**
```python
OUR_ID = 101
OUR_NAME = "dbus"
OUR_DESC = "dbus"

def postInstall(fromVersion, fromRelease, toVersion, toRelease):
    try:
        os.system("groupadd -g %d %s" % (OUR_ID, OUR_NAME))
        os.system("useradd -m -d /run/dbus -r -s /bin/false -u %d -g %d %s -c \"%s\"" %
                  (OUR_ID, OUR_ID, OUR_NAME, OUR_DESC))
    except:
        pass

def postRemove():
    try:
        os.system("userdel %s" % OUR_NAME)
        os.system("groupdel %s" % OUR_NAME)
    except:
        pass
```

**package.kdl (dönüştürülmüş):**
```kdl
package {
    post-install '(groupadd -g 101 dbus; useradd -m -d /run/dbus -r -s /bin/false -u 101 -g 101 dbus -c "dbus") 2>/dev/null || true'
    post-remove '(userdel dbus; groupdel dbus) 2>/dev/null || true'
}
```

---

## System.PackageHandler — Paket İşleyiciler

PackageHandler, XML dosya listesindeki (`<Files><Path>...</Path></Files>`)
dosyaları eşleştirme kurallarıyla işleyen bir Rust modülüdür.
Her işleyici `comar/handlers/<isim>.kdl` dosyasında tanımlanır.

### KDL Formatı

```kdl
package-handler "handler-name" {
    description "Handler description"

    setup {
        match "*.schemas"
        transform "^/etc" "/"
        run "glib-compile-schemas %DEST%"
    }

    cleanup {
        match "*.schemas"
        run "rm -f /usr/share/glib-2.0/schemas/gschemas.compiled"
    }

    post-cleanup {
        match "*.cache"
        run "rm -rf %DEST%"
    }
}
```

### Bölümler

| Bölüm | Ne Zaman Çalışır | Tipik Kullanım |
|-------|-----------------|----------------|
| `setup` | Paket kurulumunda | Derleme/önbellek oluşturma |
| `cleanup` | Paket kaldırmada | Önbellek temizleme |
| `post-cleanup` | Paket kaldırmada (en son) | Kalıntı dosyaları silme |

### Kurallar

| Kural | Açıklama | Örnek |
|-------|----------|-------|
| `match` | Eşleşen dosyaları seç | `match "*.desktop"` |
| `exclude` | Eşleşen dosyaları çıkar | `exclude "*/tests/*"` |
| `transform` | Yol dönüşümü | `transform "^/etc" "/"` |

### Özel Değişkenler

| Değişken | Açıklama |
|----------|----------|
| `%DEST%` | Eşleşen dosyanın dönüştürülmüş yolu |
| `%SRC%` | Eşleşen dosyanın orijinal (kök) yolu |

### Örnek: Baselayout

**handlers/baselayout.kdl:**
```kdl
package-handler "baselayout" {
    description "Base layout file handling"
    setup {
        match "<Path>[^<]+</Path>"
        transform "^/etc/" "/"
        run "cp /usr/share/baselayout/%DEST% /etc/%DEST%"
    }
}
```

### Mevcut Handler'lar

| Handler | Yaptığı İş |
|---------|-----------|
| `baselayout` | Baselayout dosyalarını `/usr/share/baselayout/` → `/etc/` kopyalar |
| `kmod` | Kernel modülleri için `depmod` çalıştırır |
| `python` | Python bytecode derler (`compileall`) |
| `texinfo` | Info sayfalarını günceller |
| `glib2` | GSchema şemalarını derler |
| `gtk2`/`gtk3` | GTK ikon/arayüz önbelleklerini günceller |
| `fontconfig` | Font önbelleğini yeniler |
| `desktop-file-utils` | MIME tipi veritabanını günceller |
| `gdk-pixbuf` | Pixbuf yükleyici önbelleğini yeniler |
| `gconf` | GConf şema kaydı yapar |
| `shared-mime-info` | MIME veritabanını günceller |
| `emacs` | Emacs bytecode derler |
| `dkms` | DKMS modüllerini yeniden derler |
| `mkinitramfs`/`mkinitcpio`/`initrd` | Initramfs oluşturur |
| `mudur` | Mudur (Milis) başlangıç betiklerini günceller |

---

## Python → KDL Dönüşümü

### Otomatik Dönüşüm

```bash
# Tek dosya (kuru çalıştırma)
python3 tools/comar2kdl.py --file comar/service.py --dry-run

# Tek dosya (yazma)
python3 tools/comar2kdl.py --file comar/service.py

# Tüm paketler (toplu)
python3 tools/comar2kdl.py --recursive --dir /path/to/paketler/

# Sadece fark göster
python3 tools/comar2kdl.py --file service.py --diff

# İstatistik
python3 tools/comar2kdl.py --recursive --dir /path/to/ --summary
```

### Dönüşüm Tablosu

| Python Kodu | KDL Karşılığı |
|-------------|---------------|
| `startService(command=..., args=..., donotify=True)` | `start "..."` `donotify=true` |
| `stopService(pidfile=..., donotify=True)` | `stop "kill $(cat ...)"` `donotify=true` |
| `isServiceRunning(pidfile=...)` | `status "kill -0 $(cat ...)"` |
| `fail("msg")` | `echo "msg"; exit 1` |
| `os.system("...")` | `"..."` (doğrudan komut) |
| `os.unlink(path)` | `rm -f path` |
| `os.chmod(path, mode)` | `chmod 0NNN path` |
| `os.symlink(src, dst)` | `ln -sf src dst` |
| `os.makedirs(path)` | `mkdir -p path` |
| `time.sleep(n)` | `sleep n` |
| `startDependencies(...)` | `;` (no-op) |
| `loadEnvironment()` | `. /etc/sysconfig/services` |
| `try: ... except: pass` | `(...) 2>/dev/null \|\| true` |
| `if not X:` | `[ ! X ] && { ... }` |
| `for x in Y:` | `for x in Y; do ... done` |

### Bilinmeyen Çağrılar

Dönüştürücü tanımadığı Python fonksiyonlarını uyarı olarak işaretler
ve ham kaynak metnini KDL içine yerleştirir:

```python
# Örnek: özel bir fonksiyon
configure_firewall("eth0")
# → Uyarı: "Bilinmeyen çağrı: configure_firewall"
# → KDL: configure_firewall("eth0")
```

Bu tür durumlarda KDL çıktısını el ile düzenlemeniz gerekir.

---

## Sık Kalıplar

### PID Dosyalı Servis

```kdl
service {
    type server
    description "My Service"
    start "/usr/sbin/myservice --pid-file=/run/myservice.pid" donotify=true
    stop "kill $(cat /run/myservice.pid 2>/dev/null) 2>/dev/null" donotify=true
    status "kill -0 $(cat /run/myservice.pid 2>/dev/null) 2>/dev/null"
    reload "kill -HUP $(cat /run/myservice.pid 2>/dev/null) 2>/dev/null" donotify=true
}
```

### Kullanıcı/Grup Oluşturma

```kdl
package {
    post-install '(groupadd -g 101 mygroup 2>/dev/null; useradd -r -g mygroup -s /bin/false myuser 2>/dev/null) 2>/dev/null || true'
    post-remove '(userdel myuser 2>/dev/null; groupdel mygroup 2>/dev/null) 2>/dev/null || true'
}
```

### Dosya İzinleri

```kdl
package {
    post-install "chmod 4755 /sbin/unix_chkpwd"
}
```

### Config Dosyası Kullanan Servis

```kdl
service {
    type local
    description "SSH Server"
    conf "sshd"
    start "/usr/sbin/sshd" donotify=true
    stop "kill $(cat /run/sshd.pid 2>/dev/null) 2>/dev/null" donotify=true
    status "kill -0 $(cat /run/sshd.pid 2>/dev/null) 2>/dev/null"
    reload "kill -HUP $(cat /run/sshd.pid 2>/dev/null) 2>/dev/null"
}
```

`conf "sshd"` satırı shell'de `. /etc/conf.d/sshd 2>/dev/null;` olarak
yorumlanır, böylece `$OPTS` ve benzeri değişkenler komutlarda kullanılabilir.

### Karmaşık İş Mantığı

Python'da `if`/`for`/`try` ile yapılan işlemler shell karşılıklarına
dönüştürülür. Ancak çok karmaşık durumlarda (dinamik PID dosyaları,
liste oluşturma, harici Python kütüphane çağrıları) dönüşüm sınırlıdır.
Bu durumlarda:

1. Mümkünse iş mantığını shell ile ifade edin
2. Çok karmaşıksa Python betiğini koruyun (COMAR legacy desteği ile
   çalışmaya devam eder)
3. Yardımcı shell betiklerini `/usr/share/<paket>/` altına koyup
   KDL'den çağırın

---

## Test ve Doğrulama

### Kuru Çalıştırma

```bash
python3 tools/comar2kdl.py --file comar/service.py --dry-run
```

Çıktıyı inceleyin:
- `donotify=true` doğru metodlarda var mı?
- Değişkenler doğru çözülmüş mü?
- `None` değeri sızmış mı?
- Python fonksiyon çağrıları shell içinde kalmış mı?

### Toplu Kontrol

```bash
# Tüm uyarıları listele
python3 tools/comar2kdl.py --recursive --dir /path/to/ 2>&1 | grep ⚠️

# Özet istatistik
python3 tools/comar2kdl.py --recursive --dir /path/to/ --summary
```

### Sık Karşılaşılan Sorunlar

| Sorun | Çözüm |
|-------|-------|
| `None` yolda görünüyor | Değişken çözümlenememiş, ham kaynak kullanılmalı |
| `0o700` geçersiz | `chmod`'da `0o700` → `0700` (dönüştürücü otomatik yapar) |
| Python fonksiyonları shell'de | `_get_source` fallback'i, el ile shell'e çevirin |
| Boş `{ ; }` blokları | Koşul gövdesi çözümlenememiş, el ile doldurun |
| `sh_name;` vs. | Python değişkeni çözümlenememiş, elle düzeltin |
