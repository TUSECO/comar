//! Ana D-Bus olay döngüsü — loop.c + process.c karşılığı.
//!
//! Akış:
//!   1. zbus ile system bus'a bağlan, "tr.org.luppo.comar" adını talep et
//!   2. Gelen mesajları MessageStream ile dinle
//!   3. Her mesajı Dispatch ile sınıflandır
//!   4. Core metodları (setLocale, cancel, listRunning, register, remove, introspect)
//!      doğrudan async olarak işle
//!   5. Model metodlarını spawn_blocking içinde çalıştır (Python GIL zorunluluğu)
//!   6. config_timeout saniye boyunca işlem yoksa temiz çıkış yap

use crate::{
    bus::{dispatch_message, Dispatch},
    config::Config,
    core::{
        introspect, list_application_models, list_model_applications,
        new_active_requests, new_locale_map, register, remove,
        ActiveRequests, LocaleMap,
    },
    db::ModelRegistry,
    error::{ComarError, Result},
    policy::{self, PolicyResult},
    signal_bus,
};
use rust_i18n::t;
use futures_util::StreamExt;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::Semaphore,
    time::{interval, Duration},
};
use tracing::{debug, info, warn};
use zbus::{
    message::{Message, Type as MsgType},
    Connection, MessageStream,
};

// ── Daemon çalışma durumu ─────────────────────────────────────────────────────

/// Tüm paylaşılan daemon durumu.
pub struct DaemonState {
    pub cfg:      Arc<Config>,
    pub registry: Arc<ModelRegistry>,

    /// D-Bus bağlantısı
    pub conn: Arc<Connection>,

    /// Gönderici → dil eşlemesi (setLocale tarafından güncellenir)
    pub locales: LocaleMap,

    /// Aktif istek listesi (cancel / listRunning için)
    pub requests: ActiveRequests,

    /// Eşzamanlı istek sınırı — orijinal MAX_PROC = 500
    pub semaphore: Arc<Semaphore>,

    /// Son aktivite zaman damgası (saniye, UNIX epoch)
    pub last_activity: Arc<AtomicU64>,

    /// Sinyal/iç-çağrı event kanalı (script.rs notify/call için)
    pub event_tx: Arc<tokio::sync::mpsc::UnboundedSender<signal_bus::ScriptEvent>>,
}

impl DaemonState {
    fn new(
        cfg: Arc<Config>,
        registry: Arc<ModelRegistry>,
        conn: Arc<Connection>,
        event_tx: Arc<tokio::sync::mpsc::UnboundedSender<signal_bus::ScriptEvent>>,
    ) -> Self {
        Self {
            locales:  new_locale_map(),
            requests: new_active_requests(),
            semaphore: Arc::new(Semaphore::new(500)),
            last_activity: Arc::new(AtomicU64::new(now_secs())),
            cfg,
            registry,
            conn,
            event_tx,
        }
    }

    /// Son aktivite zamanını şimdiye günceller.
    fn touch(&self) {
        self.last_activity.store(now_secs(), Ordering::Relaxed);
    }

    /// Idle timeout dolup dolmadığını kontrol eder.
    fn is_idle(&self) -> bool {
        let timeout = self.cfg.timeout;
        if timeout == 0 {
            return false; // Devre dışı
        }
        let elapsed = now_secs().saturating_sub(self.last_activity.load(Ordering::Relaxed));
        elapsed >= timeout
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Ana giriş noktası ─────────────────────────────────────────────────────────

/// D-Bus bağlantısını kur ve ana döngüyü çalıştır.
///
/// `loop_exec()` karşılığı.
pub async fn run(cfg: Arc<Config>, registry: Arc<ModelRegistry>) -> Result<()> {
    // ── D-Bus bağlantısı ──────────────────────────────────────────────────────
    let conn = Connection::system().await.map_err(ComarError::Zbus)?;

    // Bus name talep et: tr.org.luppo.comar
    conn.request_name(cfg.service_name.as_str())
        .await
        .map_err(ComarError::Zbus)?;

    info!(
        "D-Bus bağlandı: {} (unique={})",
        cfg.service_name,
        conn.unique_name().map(|n| n.to_string()).unwrap_or_default()
    );

    let conn  = Arc::new(conn);

    // ── Sinyal/call event döngüsü (signal_bus) ────────────────────────────
    let (event_tx_raw, event_rx) = tokio::sync::mpsc::unbounded_channel::<signal_bus::ScriptEvent>();
    let event_tx = Arc::new(event_tx_raw);
    let conn_events   = Arc::clone(&conn);
    let reg_events    = Arc::clone(&registry);
    let prefix_events = cfg.service_name.clone();

    tokio::spawn(signal_bus::run_event_loop(
        event_rx,
        conn_events,
        reg_events,
        prefix_events,
    ));

    let state = Arc::new(DaemonState::new(
        Arc::clone(&cfg),
        Arc::clone(&registry),
        Arc::clone(&conn),
        Arc::clone(&event_tx),
    ));

    // ── Mesaj akışı ───────────────────────────────────────────────────────────
    let mut stream = MessageStream::from(conn.as_ref());

    // ── Idle timeout kontrolü (ayrı task) ────────────────────────────────────
    let state_idle = Arc::clone(&state);
    let idle_task = tokio::spawn(async move {
        if state_idle.cfg.timeout == 0 {
            // Timeout kapalı — sonsuza dek bekle
            loop { tokio::time::sleep(Duration::from_secs(3600)).await; }
        }
        let mut ticker = interval(Duration::from_secs(5));
        loop {
            ticker.tick().await;
            if state_idle.is_idle()
                && state_idle.requests.read().await.is_empty()
            {
                info!(
                    "{}",
                    t!("status_idle_timeout", locale = "tr", timeout = state_idle.cfg.timeout)
                );
                break;
            }
        }
    });

    // ── Ctrl-C sinyali ────────────────────────────────────────────────────────
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    tokio::pin!(idle_task);

    info!("{}", t!("status_ready", locale = "tr"));

    loop {
        tokio::select! {
            // Ctrl-C
            _ = &mut ctrl_c => {
                info!("{}", t!("status_sigint"));
                break;
            }

            // Idle timeout tamamlandı
            _ = &mut idle_task => {
                break;
            }

            // Gelen D-Bus mesajı
            Some(msg_result) = stream.next() => {
                match msg_result {
                    Ok(msg) => {
                        // Yalnızca method_call'ları işle
                        if msg.message_type() != MsgType::MethodCall {
                            continue;
                        }
                        state.touch();
                        let state_clone = Arc::clone(&state);
                        let msg_arc = Arc::new(msg);
                        tokio::spawn(async move {
                            let _ = handle_message(state_clone, msg_arc).await;
                        });
                    }
                    Err(e) => {
                        warn!("{}", t!("error_messagestream", error = e));
                    }
                }
            }
        }
    }

    // Tüm aktif isteklerin bitmesini bekle
    let pending = state.requests.read().await.len();
    if pending > 0 {
        info!("{}", t!("status_pending_requests", count = pending));
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    Ok(())
}

// ── Mesaj yönlendirme ─────────────────────────────────────────────────────────

/// Tek bir D-Bus mesajını işler.
///
/// C'deki `filter_func` + `handle_message` + `handle_core_message` birleşimi.
async fn handle_message(state: Arc<DaemonState>, msg: Arc<Message>) {
    let prefix = &state.cfg.service_name;
    let dispatch = dispatch_message(&msg, prefix);

    debug!(
        "Mesaj alındı: {:?} | {:?}",
        msg.header().interface().map(|i| i.to_string()),
        msg.header().member().map(|m| m.to_string()),
    );

    match dispatch {
        // ── Introspect ───────────────────────────────────────────────────────
        Dispatch::Introspect { path } => {
            let xml = match introspect(&state.cfg, &state.registry, &path) {
                Ok(x)  => x,
                Err(e) => { reply_error(&state.conn, &msg, prefix, "Internal", &e.to_string()).await; return; }
            };
            reply_str(&state.conn, &msg, &xml).await;
        }

        // ── Core metodları ───────────────────────────────────────────────────
        Dispatch::Core { method, sender } => {
            handle_core(&state, &msg, &method, &sender).await;
        }

        // ── Model metodları ──────────────────────────────────────────────────
        Dispatch::Model { app, model, method, sender, path: _path } => {
            // Eşzamanlı istek sınırı
            let permit = match state.semaphore.clone().try_acquire_owned() {
                Ok(p)  => p,
                Err(_) => {
                    warn!("{}", t!("error_max_concurrent"));
                    reply_error(&state.conn, &msg, prefix, "Internal", &t!("error_busy", locale = "tr")).await;
                    return;
                }
            };

            // Model + method doğrula
            if !crate::db::validate_model_member(
                &state.registry,
                &model,
                &method,
                &crate::db::MemberType::Method,
            ) {
                reply_unknown_method(&state.conn, &msg).await;
                drop(permit);
                return;
            }

            // Üye tanımını al: action_id, no_reply, çıkış imzası
            let member_def = match state.registry
                .get(&model)
                .and_then(|m| m.get(&method))
            {
                Some(d) => d.clone(),
                None => {
                    reply_unknown_method(&state.conn, &msg).await;
                    drop(permit);
                    return;
                }
            };

            let action_id = member_def.action_id.clone();
            let no_reply  = member_def.no_reply;
            let sig_out   = member_def.signature_out();

            // Çağrıyı yapan istemcinin dilini çözümle (i18n için)
            let req_lang = crate::core::sender_language(&state.locales, &sender).await;

            // PolicyKit kontrolü (action_id boş değilse)
            if !action_id.is_empty() {
                match policy::check(&sender, &action_id, &state.conn).await {
                    Ok(PolicyResult::Yes) => {}
                    Ok(_) => {
                        reply_error(&state.conn, &msg, prefix, "PolicyKit", &action_id).await;
                        drop(permit);
                        return;
                    }
                    Err(_) => {
                        reply_error(&state.conn, &msg, prefix, "PolicyKit", &t!("error_policykit_generic", locale = &req_lang)).await;
                        drop(permit);
                        return;
                    }
                }
            }



            // Python betiğini spawn_blocking içinde çalıştır.
            // Python GIL, tokio async thread'leriyle uyumsuz olduğundan
            // blocking thread pool'u zorunludur.
            let conn    = Arc::clone(&state.conn);
            let msg_arc = Arc::clone(&msg);
            let cfg     = Arc::clone(&state.cfg);
            let pref    = prefix.clone();

            // D-Bus argümanlarını deserialize et
            let dbus_args: Vec<zvariant::OwnedValue> = msg
                .body()
                .deserialize::<Vec<zvariant::OwnedValue>>()
                .unwrap_or_default();

            let event_tx_clone = Arc::clone(&state.event_tx);

            let model_for_req  = model.clone();
            let method_for_req = method.clone();
            let sender_for_req = sender.clone();

            let handle = tokio::task::spawn_blocking(move || {
                // Signal/call kanalını thread-local'a yaz
                signal_bus::set_event_sender((*event_tx_clone).clone());

                // İstek bağlamını thread-local'a yaz (çözümlenmiş dil kodu dahil)
                crate::script::set_request_context(crate::script::RequestContext {
                    path:         msg_arc.header().path().map(|p| p.to_string()).unwrap_or_default(),
                    interface:    msg_arc.header().interface().map(|i| i.to_string()).unwrap_or_default(),
                    member:       msg_arc.header().member().map(|m| m.to_string()).unwrap_or_default(),
                    sender:       msg_arc.header().sender().map(|s| s.to_string()).unwrap_or_default(),
                    lang:         req_lang.clone(),
                    data_dir:     cfg.dir_data.to_string_lossy().to_string(),
                    iface_prefix: cfg.service_name.clone(),
                });

                let result = match crate::services::dispatch(&model, &method, &dbus_args, &req_lang) {
                    crate::services::ServiceDispatch::Native(res) => res,
                    crate::services::ServiceDispatch::Passthrough => {
                        crate::script::execute(
                            &cfg,
                            Some(&app),
                            &model,
                            &method,
                            &dbus_args,
                        )
                    }
                };

                crate::script::clear_request_context();
                signal_bus::clear_event_sender();

                let rt = tokio::runtime::Handle::current();
                rt.block_on(async move {
                    match result {
                        Ok(values) => {
                            // no_reply metodları için yanıt gönderilmez
                            if no_reply {
                                // İstemci yanıt beklemediğinden hiçbir şey gönderme
                            } else if values.is_empty() || sig_out.is_empty() {
                                // Void dönüş: boş başarı yanıtı
                                if let Ok(reply) = Message::method_reply(&*msg_arc).and_then(|b| b.build(&())) {
                                    let _ = conn.send(&reply).await;
                                }
                            } else if values.len() == 1 {
                                // Tek değer
                                if let Ok(reply) = Message::method_reply(&*msg_arc).and_then(|b| b.build(&values[0])) {
                                    let _ = conn.send(&reply).await;
                                }
                            } else {
                                // Çoklu değer — yapı (structure) olarak gönder
                                let mut sb = zvariant::StructureBuilder::new();
                                for v in &values {
                                    if let Ok(owned) = v.try_clone() {
                                        sb.push_value(owned.into());
                                    }
                                }
                                if let Ok(reply) = Message::method_reply(&*msg_arc).and_then(|b| b.build(&sb.build())) {
                                    let _ = conn.send(&reply).await;
                                }
                            }
                        }
                        Err(ComarError::PolicyKit(action)) => {
                            reply_error(&conn, &msg_arc, &pref, "PolicyKit", &action).await;
                        }
                        Err(e) => {
                            let name = match &e {
                                ComarError::Missing(_) => "Missing",
                                ComarError::Invalid(_) => "Invalid",
                                ComarError::Script(_)  => "Script",
                                _                      => "Internal",
                            };
                            reply_error(&conn, &msg_arc, &pref, name, &e.to_string()).await;
                        }
                    }
                    drop(permit);
                });
            });

            // Aktif istek listesine ekle
            let req = crate::core::ActiveRequest {
                sender:    sender_for_req,
                interface: format!("{}.{}", prefix, model_for_req),
                method:    method_for_req,
                handle,
            };
            state.requests.write().await.push(req);
        }

        Dispatch::Unknown => {
            reply_unknown_method(&state.conn, &msg).await;
        }
    }

    // Tamamlanan istekleri temizle
    cleanup_finished(&state).await;
}

// ── Core metod işleyicileri ───────────────────────────────────────────────────

/// Core arayüzü metodlarını işler.
///
/// `handle_core_message()` karşılığı.
async fn handle_core(
    state:  &Arc<DaemonState>,
    msg:    &Arc<Message>,
    method: &str,
    sender: &str,
) {
    let prefix = &state.cfg.service_name;

    match method {
        // setLocale(s locale) → b
        // Göndericinin dilini kaydeder
        "setLocale" => {
            if let Some(locale) = extract_string_arg(msg) {
                state.locales.write().await.insert(sender.to_string(), locale);
                reply_bool(&state.conn, msg, true).await;
            } else {
                reply_error(&state.conn, msg, prefix, "Invalid", &t!("error_invalid_arg", locale = "tr", arg = "locale")).await;
            }
        }

        // cancel(s method_name) → i
        // Göndericiye ait, verilen method adındaki aktif istekleri iptal eder.
        // method_name = "*" veya boş ise gönderici adına ait tüm istekler iptal edilir.
        "cancel" => {
            let method_name = extract_string_arg(msg).unwrap_or_default();
            let cancel_all  = method_name.is_empty() || method_name == "*";

            let mut killed = 0i32;
            let mut requests = state.requests.write().await;
            requests.retain(|req| {
                let sender_match = req.sender == sender;
                let method_match = cancel_all || req.method == method_name;
                if sender_match && method_match {
                    req.handle.abort();
                    killed += 1;
                    false
                } else {
                    true
                }
            });
            debug!(
                "cancel: {} istek iptal edildi (sender={}, method={})",
                killed, sender, method_name
            );
            reply_int(&state.conn, msg, killed).await;
        }

        // listRunning(b all) → as
        // Çalışan metodları listeler
        "listRunning" => {
            let all = extract_bool_arg(msg).unwrap_or(false);
            let requests = state.requests.read().await;
            let methods: Vec<String> = requests
                .iter()
                .filter(|r| all || r.sender == sender)
                .map(|r| format!("{}.{}", r.interface, r.method))
                .collect();
            reply_string_array(&state.conn, msg, &methods).await;
        }

        // register(s app, s model, s script) → (NoReply)
        "register" => {
            if let Some((app, model, script)) = extract_three_strings(msg) {
                match register(&state.cfg, &app, &model, &script) {
                    Ok(_)  => { reply_bool(&state.conn, msg, true).await; }
                    Err(e) => { reply_error(&state.conn, msg, prefix, "Internal", &e.to_string()).await; }
                }
            } else {
                reply_error(&state.conn, msg, prefix, "Invalid", &t!("error_invalid_arg", locale = "tr", arg = "app, model, script")).await;
            }
        }

        // remove(s app) → (NoReply)
        "remove" => {
            if let Some(app) = extract_string_arg(msg) {
                match remove(&state.cfg, &app) {
                    Ok(_)  => { reply_bool(&state.conn, msg, true).await; }
                    Err(e) => { reply_error(&state.conn, msg, prefix, "Internal", &e.to_string()).await; }
                }
            } else {
                reply_error(&state.conn, msg, prefix, "Invalid", &t!("error_invalid_arg", locale = "tr", arg = "app")).await;
            }
        }

        // listModelApplications(s model) → as
        "listModelApplications" => {
            if let Some(model) = extract_string_arg(msg) {
                match list_model_applications(&state.cfg, &model) {
                    Ok(apps) => reply_string_array(&state.conn, msg, &apps).await,
                    Err(e)   => reply_error(&state.conn, msg, prefix, "Internal", &e.to_string()).await,
                }
            } else {
                reply_error(&state.conn, msg, prefix, "Invalid", &t!("error_invalid_arg", locale = "tr", arg = "model")).await;
            }
        }

        // listApplicationModels(s app) → as
        "listApplicationModels" => {
            if let Some(app) = extract_string_arg(msg) {
                match list_application_models(&state.cfg, &app) {
                    Ok(models) => reply_string_array(&state.conn, msg, &models).await,
                    Err(e)     => reply_error(&state.conn, msg, prefix, "Internal", &e.to_string()).await,
                }
            } else {
                reply_error(&state.conn, msg, prefix, "Invalid", &t!("error_invalid_arg", locale = "tr", arg = "app")).await;
            }
        }

        other => {
            warn!("{}", t!("error_unknown_core_method_warn", method = other));
            reply_unknown_method(&state.conn, &msg).await;
        }
    }
}

// ── Tamamlanan istekleri temizle ──────────────────────────────────────────────

async fn cleanup_finished(state: &Arc<DaemonState>) {
    let mut requests = state.requests.write().await;
    requests.retain(|r| !r.handle.is_finished());
}

// ── D-Bus cevap yardımcıları ──────────────────────────────────────────────────
// bus.c'deki bus_reply_*, bus_send karşılıkları

async fn reply_error(conn: &Connection, msg: &Message, prefix: &str, name: &str, text: &str) {
    let err_name = format!("{}.{}", prefix, name);
    if let Ok(builder) = Message::method_error(msg, err_name.as_str()) {
        if let Ok(reply) = builder.build(&text) {
            let _ = conn.send(&reply).await;
        }
    }
}

async fn reply_unknown_method(conn: &Connection, msg: &Message) {
    let method = msg.header().member().map(|m| m.to_string()).unwrap_or_default();
    let iface  = msg.header().interface().map(|i| i.to_string()).unwrap_or_default();
    let sig    = msg.body().signature().map(|s| s.to_string()).unwrap_or_default();
    let text   = format!(
        "Method \"{}\" with signature \"{}\" on interface \"{}\" doesn't exist",
        method, sig, iface
    );
    if let Ok(builder) = Message::method_error(msg, "org.freedesktop.DBus.Error.UnknownMethod") {
        if let Ok(reply) = builder.build(&text) {
            let _ = conn.send(&reply).await;
        }
    }
}

async fn reply_str(conn: &Connection, msg: &Message, s: &str) {
    let owned = s.to_string();
    if let Ok(reply) = Message::method_reply(msg).and_then(|b| b.build(&owned)) {
        let _ = conn.send(&reply).await;
    }
}

async fn reply_bool(conn: &Connection, msg: &Message, b: bool) {
    if let Ok(reply) = Message::method_reply(msg).and_then(|bld| bld.build(&b)) {
        let _ = conn.send(&reply).await;
    }
}

async fn reply_int(conn: &Connection, msg: &Message, i: i32) {
    if let Ok(reply) = Message::method_reply(msg).and_then(|b| b.build(&i)) {
        let _ = conn.send(&reply).await;
    }
}

async fn reply_string_array(conn: &Connection, msg: &Message, arr: &[String]) {
    let refs: Vec<&str> = arr.iter().map(|s| s.as_str()).collect();
    if let Ok(reply) = Message::method_reply(msg).and_then(|b| b.build(&refs.as_slice())) {
        let _ = conn.send(&reply).await;
    }
}

// ── D-Bus argüman ayrıştırma ──────────────────────────────────────────────────

fn extract_string_arg(msg: &Message) -> Option<String> {
    msg.body().deserialize::<(String,)>().ok().map(|(s,)| s)
}

fn extract_bool_arg(msg: &Message) -> Option<bool> {
    msg.body().deserialize::<(bool,)>().ok().map(|(b,)| b)
}

fn extract_three_strings(msg: &Message) -> Option<(String, String, String)> {
    msg.body().deserialize::<(String, String, String)>().ok()
}
