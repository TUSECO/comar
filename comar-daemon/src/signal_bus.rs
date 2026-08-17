//! Sinyal ve iç çağrı köprüsü.
//!
//! Python betikleri `notify()` ve `call()` CSL fonksiyonlarını çağırdığında
//! bu istekler async D-Bus işlemlerini tetikler. Ancak `spawn_blocking` içinde
//! çalışan Python kodu doğrudan `async` çağrı yapamaz.
//!
//! Çözüm: thread-local bir `mpsc::Sender` ile istekler kuyruğa alınır,
//! `process.rs` bu kuyruğu dinleyerek D-Bus üzerinden gönderir.
//!
//! # Akış
//! ```text
//! Python betik (blocking thread)
//!   └── notify("System.Service", "Changed", (pkg, "started"))
//!         └── SIGNAL_TX.send(SignalRequest { ... })
//!               └── process.rs signal_loop'u alır
//!                     └── conn.emit_signal(...)
//! ```

use crate::db::ModelRegistry;

use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::debug;
use zvariant::OwnedValue;

// ── Mesaj tipleri ─────────────────────────────────────────────────────────────

/// Python `notify()` tarafından gönderilen sinyal isteği.
///
/// `c_notify()` → `bus_signal()` karşılığı.
#[derive(Debug)]
pub struct SignalRequest {
    /// D-Bus nesne yolu  (örn. `/package/apache`)
    pub path:      String,
    /// Model adı        (örn. `System.Service`)
    pub model:     String,
    /// Sinyal adı       (örn. `Changed`)
    pub signal:    String,
    /// Sinyal argümanları
    pub args:      Vec<OwnedValue>,
}

/// Python `call()` tarafından gönderilen iç çağrı isteği.
///
/// `c_call()` → `bus_call()` → `bus_execute()` karşılığı.
#[derive(Debug)]
pub struct CallRequest {
    /// Hedef uygulama  (örn. `apache`)
    pub app:     String,
    /// Model adı       (örn. `System.Service`)
    pub model:   String,
    /// Metod adı       (örn. `start`)
    pub method:  String,
    /// Argümanlar
    pub args:    Vec<OwnedValue>,
    /// Zaman aşımı (-1 = varsayılan)
    #[allow(dead_code)]
    pub timeout: i32,
    /// Dil kodu
    #[allow(dead_code)]
    pub lang:    String,
}

/// İç çağrı yanıtı.
#[derive(Debug)]
pub struct CallReply {
    #[allow(dead_code)]
    pub values: Vec<OwnedValue>,
    #[allow(dead_code)]
    pub error:  Option<String>,
}

/// Betik motorundan gelen tüm olaylar.
#[derive(Debug)]
pub enum ScriptEvent {
    Signal(SignalRequest),
    Call(CallRequest),
}

// ── Thread-local göndericiler ─────────────────────────────────────────────────
//
// Her `spawn_blocking` çağrısı başlamadan önce bu sender'lar ayarlanır.
// Python CSL built-in'leri bu sender'ları kullanır.

use std::cell::RefCell;

thread_local! {
    /// Sinyal/call olaylarını async döngüye iletmek için sender.
    static EVENT_TX: RefCell<Option<mpsc::UnboundedSender<ScriptEvent>>> =
        RefCell::new(None);
}

/// Thread-local event sender'ı ayarlar.
pub fn set_event_sender(tx: mpsc::UnboundedSender<ScriptEvent>) {
    EVENT_TX.with(|t| *t.borrow_mut() = Some(tx));
}

/// Thread-local event sender'ı temizler.
pub fn clear_event_sender() {
    EVENT_TX.with(|t| *t.borrow_mut() = None);
}

/// Sinyal isteği gönderir. CSL `notify()` tarafından çağrılır.
pub fn send_signal(req: SignalRequest) -> bool {
    EVENT_TX.with(|t| {
        if let Some(tx) = t.borrow().as_ref() {
            tx.send(ScriptEvent::Signal(req)).is_ok()
        } else {
            false
        }
    })
}

/// İç çağrı isteği gönderir ve yanıtı bekler. CSL `call()` tarafından çağrılır.
pub fn send_call_and_wait(req: CallRequest) -> CallReply {
    let sent = EVENT_TX.with(|t| {
        if let Some(tx) = t.borrow().as_ref() {
            tx.send(ScriptEvent::Call(req)).is_ok()
        } else {
            false
        }
    });

    if !sent {
        return CallReply {
            values: vec![],
            error:  Some("Event sender hazır değil".into()),
        };
    }
    // Yanıt yoksa boş döndür (call() async, sync wait desteklenmiyor)
    CallReply { values: vec![], error: None }
}

// ── Async olay döngüsü ────────────────────────────────────────────────────────

/// Sinyal ve iç çağrı olaylarını işleyen async döngü.
///
/// `process.rs` içindeki ana döngüyle paralel çalışır.
pub async fn run_event_loop(
    mut rx:      mpsc::UnboundedReceiver<ScriptEvent>,
    conn:        Arc<zbus::Connection>,
    registry:    Arc<ModelRegistry>,
    iface_prefix: String,
) {
    while let Some(event) = rx.recv().await {
        match event {
            ScriptEvent::Signal(req) => {
                handle_signal(req, &conn, &registry, &iface_prefix).await;
            }
            ScriptEvent::Call(req) => {
                handle_call(req, &conn, &iface_prefix).await;
            }
        }
    }
}

// ── Sinyal gönderme ──────────────────────────────────────────────────────────

/// D-Bus sinyali gönderir.
///
/// `bus_signal()` karşılığı.
async fn handle_signal(
    req:          SignalRequest,
    conn:         &zbus::Connection,
    registry:     &ModelRegistry,
    iface_prefix: &str,
) {
    // Arayüz adını oluştur: tr.org.luppo.comar.System.Service
    let interface = if req.model == "Core" {
        iface_prefix.to_string()
    } else {
        format!("{}.{}", iface_prefix, req.model)
    };

    // Sinyal tanımını doğrula
    let sig_out = registry
        .get(&req.model)
        .and_then(|m| m.get(&req.signal))
        .map(|d| d.signature_out())
        .unwrap_or_default();

    debug!(
        "Sinyal gönderiliyor: {}.{} path={} sig={}",
        interface, req.signal, req.path, sig_out
    );

    // zbus ile sinyal gönder
    // ObjectServer üzerinden değil, doğrudan Message ile
    match build_signal_message(&req.path, &interface, &req.signal, &req.args) {
        Ok(msg) => {
            if let Err(e) = conn.send(&msg).await {
                tracing::error!("Sinyal gönderilemedi: {}", e);
            }
        }
        Err(e) => {
            tracing::error!("Sinyal mesajı oluşturulamadı: {}", e);
        }
    }
}

/// D-Bus sinyal mesajı oluşturur.
fn build_signal_message(
    path:      &str,
    interface: &str,
    signal:    &str,
    args:      &[OwnedValue],
) -> std::result::Result<zbus::message::Message, zbus::Error> {
    use zbus::message::Message;

    // Argümansız sinyal
    if args.is_empty() {
        return Message::signal(path, interface, signal)
            .and_then(|b| b.build(&()));
    }

    // Tek argüman
    if args.len() == 1 {
        return Message::signal(path, interface, signal)
            .and_then(|b| b.build(&args[0]));
    }

    // Çoklu argüman — struct olarak gönder
    let mut builder = zvariant::StructureBuilder::new();
    for v in args {
        if let Ok(owned) = v.try_clone() {
            builder.push_value(owned.into());
        }
    }
    let structure = builder.build();
    Message::signal(path, interface, signal)
        .and_then(|b| b.build(&structure))
}

// ── İç çağrı ─────────────────────────────────────────────────────────────────

/// Başka bir ÇOMAR metodunu D-Bus üzerinden çağırır.
///
/// `bus_call()` + `bus_execute()` karşılığı.
async fn handle_call(
    req:          CallRequest,
    conn:         &zbus::Connection,
    iface_prefix: &str,
) {
    let path      = format!("/package/{}", req.app);
    let interface = format!("{}.{}", iface_prefix, req.model);

    debug!("İç çağrı: {}.{} path={}", interface, req.method, path);

    // zbus call_method: argümanlar zvariant::Value olarak serileştiriliyor.
    // Argüman yoksa boş tuple, varsa OwnedValue slice kullan.
    let call_result = if req.args.is_empty() {
        conn.call_method(
            Some(iface_prefix),
            path.as_str(),
            Some(interface.as_str()),
            req.method.as_str(),
            &(),
        )
        .await
    } else {
        // OwnedValue'lar zaten zvariant serializable; Structure olarak geçir
        conn.call_method(
            Some(iface_prefix),
            path.as_str(),
            Some(interface.as_str()),
            req.method.as_str(),
            &{
                let mut builder = zvariant::StructureBuilder::new();
                for v in &req.args {
                    if let Ok(owned) = v.try_clone() {
                        builder.push_value(owned.into());
                    }
                }
                builder.build()
            },
        )
        .await
    };

    match call_result {
        Ok(_) => debug!("İç çağrı tamamlandı: {}.{}", interface, req.method),
        Err(e) => debug!("İç çağrı hatası: {}", e),
    }
}
