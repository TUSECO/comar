//! ComarLink — Python comar.Link sınıfının tam Rust karşılığı.

use crate::error::{ApiError, Result};
use futures_util::StreamExt;
use std::sync::Arc;
use tracing::debug;
use zbus::{
    message::Message,
    zvariant::OwnedValue,
    Connection, MessageStream,
};

pub struct ComarLink {
    pub address:   String,
    pub interface: String,
    conn: Arc<Connection>,
}

impl ComarLink {
    /// Sistem D-Bus'ına bağlan. `Link.__init__()` karşılığı.
    pub async fn new(service_name: Option<&str>) -> Result<Self> {
        let address = service_name
            .unwrap_or("tr.org.luppo.comar")
            .to_string();
        let conn = Connection::system()
            .await
            .map_err(|e| ApiError::Connection(e.to_string()))?;
        debug!("ComarLink bağlandı: {}", address);
        Ok(Self { interface: address.clone(), address, conn: Arc::new(conn) })
    }

    /// Unix soket adresine bağlan (test için). `Link.__init__(socket=...)` karşılığı.
    pub async fn new_with_socket(socket_path: &str) -> Result<Self> {
        // zbus 4.x: connection builder unix soket için
        let conn = zbus::connection::Builder::unix_stream(
                tokio::net::UnixStream::connect(socket_path)
                    .await
                    .map_err(|e| ApiError::Connection(e.to_string()))?,
            )
            .build()
            .await
            .map_err(|e| ApiError::Connection(e.to_string()))?;
        Ok(Self {
            address:   "tr.org.luppo.comar".into(),
            interface: "tr.org.luppo.comar".into(),
            conn: Arc::new(conn),
        })
    }

    // ── Core metodları ────────────────────────────────────────────────────

    /// Yerel ayarı ÇOMAR'a bildir. `Link.setLocale()` karşılığı.
    pub async fn set_locale(&self, locale: &str) -> Result<()> {
        // &str DST (unsized) olduğundan String'e dönüştürerek geçiriyoruz.
        let s = locale.to_string();
        self.call_method("/", &self.interface, "setLocale", &s).await?;
        Ok(())
    }

    /// Çalışan metodları listele. `Link.listRunning(all=True)` karşılığı.
    pub async fn list_running(&self, all: bool) -> Result<Vec<String>> {
        let reply = self
            .call_method("/", &self.interface, "listRunning", &all)
            .await?;
        reply
            .body()
            .deserialize::<(Vec<String>,)>()
            .map(|(v,)| v)
            .map_err(|e| ApiError::Call(e.to_string()))
    }

    /// Çalışan bir metodu iptal et. `Link.cancel()` karşılığı.
    pub async fn cancel(&self) -> Result<i32> {
        let method = "*".to_string();
        let reply = self
            .call_method("/", &self.interface, "cancel", &method)
            .await?;
        reply
            .body()
            .deserialize::<(i32,)>()
            .map(|(n,)| n)
            .map_err(|e| ApiError::Call(e.to_string()))
    }

    /// Paketi modelle kaydet. `Link.register(app, model, script)` karşılığı.
    pub async fn register(&self, app: &str, model: &str, script: &str) -> Result<()> {
        let args = (app.to_string(), model.to_string(), script.to_string());
        self.call_method("/", &self.interface, "register", &args).await?;
        Ok(())
    }

    /// Paketin tüm kayıtlarını sil. `Link.remove(app)` karşılığı.
    pub async fn remove(&self, app: &str) -> Result<()> {
        let s = app.to_string();
        self.call_method("/", &self.interface, "remove", &s).await?;
        Ok(())
    }

    /// Modeli uygulayan paketleri listele.
    pub async fn list_model_applications(&self, model: &str) -> Result<Vec<String>> {
        let s = model.to_string();
        let reply = self
            .call_method("/", &self.interface, "listModelApplications", &s)
            .await?;
        reply
            .body()
            .deserialize::<(Vec<String>,)>()
            .map(|(v,)| v)
            .map_err(|e| ApiError::Call(e.to_string()))
    }

    // ── Model metod çağrısı ───────────────────────────────────────────────

    /// Belirli uygulama + model üzerinde metod çağır. `Call.call()` karşılığı.
    pub async fn call<B>(&self, app: &str, model: &str, method: &str, body: &B)
        -> Result<Message>
    where
        B: zvariant::DynamicType + serde::Serialize,
    {
        let path      = format!("/package/{}", app);
        let interface = format!("{}.{}", self.interface, model);
        self.call_method(&path, &interface, method, body).await
    }

    // ── Sinyal dinleme ────────────────────────────────────────────────────

    /// Modelden gelen sinyalleri dinle. `Link.listenSignals(model, handler)` karşılığı.
    ///
    /// Handler: `(package: String, signal: String, args: Vec<OwnedValue>)`
    pub async fn listen_signals<F>(
        &self,
        model:   &str,
        handler: F,
    ) -> Result<()>
    where
        F: Fn(String, String, Vec<OwnedValue>) + Send + 'static,
    {
        let interface       = format!("{}.{}", self.interface, model);
        let mut stream      = MessageStream::from(self.conn.as_ref());
        let interface_owned = interface.clone();

        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                if msg.message_type() != zbus::message::Type::Signal {
                    continue;
                }

                // zbus 4.x: deprecated msg.interface()/path()/member() yerine
                // msg.header() üzerinden erişim kullanılır.
                let hdr = msg.header();

                let msg_iface = hdr
                    .interface()
                    .map(|i| i.to_string())
                    .unwrap_or_default();

                if msg_iface != interface_owned {
                    continue;
                }

                let path = hdr
                    .path()
                    .map(|p| p.to_string())
                    .unwrap_or_default();

                let signal = hdr
                    .member()
                    .map(|m| m.to_string())
                    .unwrap_or_default();

                let package = path
                    .strip_prefix("/package/")
                    .unwrap_or("")
                    .to_string();

                if package.is_empty() {
                    continue;
                }

                let args: Vec<OwnedValue> = msg
                    .body()
                    .deserialize::<Vec<OwnedValue>>()
                    .unwrap_or_default();

                handler(package, signal, args);
            }
        });

        Ok(())
    }

    // ── İç yardımcı ──────────────────────────────────────────────────────

    /// Genel D-Bus metod çağrısı.
    ///
    /// `B` Sized olmalı; `&str` gibi DST tipleri için çağıran tarafta
    /// önce `String`'e dönüştürün.
    async fn call_method<B>(
        &self,
        path:      &str,
        interface: &str,
        method:    &str,
        body:      &B,
    ) -> Result<Message>
    where
        B: zvariant::DynamicType + serde::Serialize,
    {
        self.conn
            .call_method(
                Some(self.address.as_str()),
                path,
                Some(interface),
                method,
                body,
            )
            .await
            .map_err(|e| ApiError::Call(e.to_string()))
    }
}

// ── Testler ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[test]
    fn address_format_correct() {
        let addr = "tr.org.luppo.comar";
        assert!(addr.starts_with("tr.org.luppo"));
    }

    #[test]
    fn interface_for_model_format() {
        let iface = format!("{}.{}", "tr.org.luppo.comar", "System.Service");
        assert_eq!(iface, "tr.org.luppo.comar.System.Service");
    }

    #[test]
    fn package_path_format() {
        assert_eq!(format!("/package/{}", "apache"), "/package/apache");
    }

    #[test]
    fn package_path_strip() {
        assert_eq!(
            "/package/nginx".strip_prefix("/package/").unwrap_or(""),
            "nginx"
        );
    }

    #[test]
    fn empty_package_filtered() {
        assert!("/package/".strip_prefix("/package/").unwrap_or("").is_empty());
    }
}
