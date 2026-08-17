# ÇOMAR Rust — Makefile

# Ayarlar
PREFIX      ?= /usr
SBINDIR     ?= $(PREFIX)/sbin
DATADIR     ?= $(PREFIX)/share
SYSCONFDIR  ?= /etc
VARDIR      ?= /var/db/comar3
LOGDIR      ?= /var/log/comar3

# Dizinler
DBUS_SYSDIR     = $(DATADIR)/dbus-1/system.d
DBUS_SVCDIR     = $(DATADIR)/dbus-1/system-services
POLKIT_ACTDIR   = $(DATADIR)/polkit-1/actions
SYSTEMD_UNITDIR = /usr/lib/systemd/system
LOCALES_DIR     = $(DATADIR)/comar/locales

# Araçlar
CARGO ?= cargo
PYTHON ?= python3

# Platforma uygun kütüphane uzantısını bul
UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Darwin)
    LIB_EXT := dylib
else
    LIB_EXT := so
endif

# Hedefler
BINARY    = target/release/comar
SO_SRC    = target/release/lib_comar.$(LIB_EXT)

# Python site-packages dizinini otomatik bul
PYSITE    ?= $(shell $(PYTHON) -c "import site; print(site.getsitepackages()[0])" 2>/dev/null)
PYSUFFIX  ?= $(shell $(PYTHON)-config --extension-suffix 2>/dev/null || echo .so)
SO_DEST   = $(DESTDIR)$(PYSITE)/comar/_comar$(PYSUFFIX)

.PHONY: all help build build-release build-daemon build-api build-py dev install install-py uninstall clean test check fmt clippy doc bench

# Varsayılan hedef
all: build-release

help:
	@echo "ÇOMAR Rust Makefile"
	@echo ""
	@echo "Kullanım:"
	@echo "  make              → Release sürümünü derle (daemon + api + Python binding)"
	@echo "  make build        → 'make' ile aynı"
	@echo "  make build-daemon → Yalnızca comar daemon binary'sini derle"
	@echo "  make build-api    → Yalnızca comar-api library'sini derle"
	@echo "  make build-py     → Yalnızca comar-py Python binding'ini derle"
	@echo "  make dev          → Geliştirici modunda derle (debug)"
	@echo "  make test         → Testleri çalıştır"
	@echo "  make check        → Cargo check çalıştır"
	@echo "  make fmt          → Kodu formatla"
	@echo "  make clippy       → Statik analiz yap (clippy)"
	@echo "  make doc          → Dokümantasyon oluştur"
	@echo "  make bench        → Benchmark testlerini çalıştır"
	@echo "  make install      → Sisteme kur (daemon + Python binding + dbus + polkit + systemd + locales)"
	@echo "  make install-py   → Yalnızca Python binding'i kur"
	@echo "  make uninstall    → Sistemden kaldır"
	@echo "  make clean        → Derleme dosyalarını temizle"

build: build-release

build-release: build-daemon build-api build-py

build-daemon:
	$(CARGO) build --release -p comar-daemon --bin comar

build-api:
	$(CARGO) build --release -p comar-api

build-py:
	$(CARGO) build --release -p comar-py

dev:
	$(CARGO) build --workspace

test:
	$(CARGO) test --workspace

check:
	$(CARGO) check --workspace

fmt:
	$(CARGO) fmt --all

clippy:
	$(CARGO) clippy --workspace -- -D warnings

doc:
	$(CARGO) doc --workspace --no-deps

bench:
	$(CARGO) bench --workspace

install: build-release
	# Dizinleri oluştur
	install -dm755 $(DESTDIR)$(SBINDIR)
	install -dm755 $(DESTDIR)$(VARDIR)/models
	install -dm755 $(DESTDIR)$(VARDIR)/modules
	install -dm755 $(DESTDIR)$(VARDIR)/scripts
	install -dm755 $(DESTDIR)$(VARDIR)/apps
	install -dm755 $(DESTDIR)$(LOGDIR)
	install -dm755 $(DESTDIR)$(LOCALES_DIR)

	# Binary
	install -m755 $(BINARY) $(DESTDIR)$(SBINDIR)/comar

	# D-Bus yapılandırması
	install -Dm644 data/dbus-1/system.d/tr.org.luppo.comar.conf \
		$(DESTDIR)$(DBUS_SYSDIR)/tr.org.luppo.comar.conf
	install -Dm644 data/dbus-1/system-services/tr.org.luppo.comar.service \
		$(DESTDIR)$(DBUS_SVCDIR)/tr.org.luppo.comar.service

	# PolicyKit action dosyaları
	install -dm755 $(DESTDIR)$(POLKIT_ACTDIR)
	install -m644 data/polkit-1/actions/*.policy \
		$(DESTDIR)$(POLKIT_ACTDIR)/

	# systemd servis dosyası
	install -Dm644 data/systemd/system/comar.service \
		$(DESTDIR)$(SYSTEMD_UNITDIR)/comar.service

	# Locales
	install -m644 locales/*.yml $(DESTDIR)$(LOCALES_DIR)/

	# Python binding
	$(MAKE) install-py

	@echo ""
	@echo "ÇOMAR başarıyla kuruldu."

install-py: build-py
	@if [ -z "$(PYSITE)" ]; then \
		echo "HATA: Python site-packages dizini bulunamadı."; \
		exit 1; \
	fi
	install -dm755 $(DESTDIR)$(PYSITE)/comar
	install -m755 $(SO_SRC) $(SO_DEST)
	install -m644 comar-py/comar/*.py $(DESTDIR)$(PYSITE)/comar/
	@echo "Python binding kuruldu: $(DESTDIR)$(PYSITE)/comar"

uninstall:
	rm -f $(DESTDIR)$(SBINDIR)/comar
	rm -f $(DESTDIR)$(DBUS_SYSDIR)/tr.org.luppo.comar.conf
	rm -f $(DESTDIR)$(DBUS_SVCDIR)/tr.org.luppo.comar.service
	rm -f $(DESTDIR)$(POLKIT_ACTDIR)/tr.org.luppo.comar*.policy
	rm -f $(DESTDIR)$(SYSTEMD_UNITDIR)/comar.service
	rm -rf $(DESTDIR)$(LOCALES_DIR)
	rm -rf $(DESTDIR)$(PYSITE)/comar
	@echo "ÇOMAR sistemden kaldırıldı."

clean:
	$(CARGO) clean
