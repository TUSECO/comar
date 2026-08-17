//! PolicyKit entegrasyonu — policy.c karşılığı.
//!
//! org.freedesktop.PolicyKit1.Authority.CheckAuthorization D-Bus çağrısı.
//! UID = 0 (root) ise PolicyKit atlanır — orijinal C davranışıyla birebir.

use crate::error::{ComarError, Result};
use tracing::{debug, error};
use zbus::{
    proxy,
    zvariant::{OwnedValue, Value},
    Connection,
};
use std::collections::HashMap;

// ── PolicyKit D-Bus proxy ─────────────────────────────────────────────────────

/// org.freedesktop.PolicyKit1.Authority arayüzünün D-Bus proxy'si.
///
/// zbus `proxy` macro'su ile otomatik oluşturulur.
#[proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority"
)]
trait PolicyKitAuthority {
    /// CheckAuthorization çağrısı.
    ///
    /// subject:      (type, {details})   — örn. ("system-bus-name", {"name": sender})
    /// action_id:    PolicyKit action ID
    /// details:      ek ayrıntılar (boş dict)
    /// flags:        1 = AllowUserInteraction
    /// cancellation_id: ""
    ///
    /// Döndürür: (is_authorized: bool, is_challenge: bool, details: {})
    fn check_authorization(
        &self,
        subject: (String, HashMap<String, OwnedValue>),
        action_id: &str,
        details: HashMap<String, String>,
        flags: u32,
        cancellation_id: &str,
    ) -> zbus::Result<(bool, bool, HashMap<String, String>)>;
}

// ── Sorgu sonucu ──────────────────────────────────────────────────────────────

/// PolicyKit sorgu sonucu.
///
/// C'deki `POLICY_YES / POLICY_NO` karşılığı.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum PolicyResult {
    /// Yetkilendirildi
    Yes,
    /// Reddedildi
    No,
    /// Kullanıcıdan etkileşimli onay gerekiyor
    Challenge,
}

// ── Ana fonksiyon ─────────────────────────────────────────────────────────────

/// Gönderenin belirtilen action için yetkisini kontrol eder.
///
/// `policy_check()` karşılığı.
///
/// Akış:
/// 1. Boş action_id → her zaman `Yes`
/// 2. Gönderenin UID'ini sorgula
/// 3. UID = 0 (root) → `Yes` (PolicyKit atlanır)
/// 4. PolicyKit CheckAuthorization çağrısı
pub async fn check(
    sender: &str,
    action_id: &str,
    connection: &Connection,
) -> Result<PolicyResult> {
    // Boş action_id → kontrol gerekmez
    if action_id.is_empty() {
        return Ok(PolicyResult::Yes);
    }

    let mut action_id_used = action_id.to_string();
    // COMAR 2.0 uyumluluğu: Eski policy ID'leri genellikle "System.Service.set" şeklinde büyük harfler içerirdi.
    if action_id_used.starts_with("tr.org.luppo.comar.") && action_id_used.chars().any(|c| c.is_uppercase()) {
        action_id_used = action_id_used.to_lowercase();
        debug!("COMAR 2.0 policy ID tespit edildi, güncellendi: {}", action_id_used);
    }

    // Gönderenin UID'ini al — zbus'a erişim için ayrı bağlantı
    // (orijinal C: dbus_bus_get_private + dbus_bus_get_unix_user)
    let uid = get_sender_uid(connection, sender).await?;

    // Root ise PolicyKit'i atla (policy.c satır 49–52)
    if uid == 0 {
        debug!("UID=0 (root), PolicyKit atlandı: action={}", action_id);
        return Ok(PolicyResult::Yes);
    }

    debug!(
        "PolicyKit sorgusu: sender={} uid={} action={}",
        sender, uid, action_id
    );

    // PolicyKit proxy'si oluştur
    let proxy = PolicyKitAuthorityProxy::new(connection)
        .await
        .map_err(ComarError::Zbus)?;

    // Subject: ("system-bus-name", {"name": sender_bus_name})
    let mut subject_details: HashMap<String, OwnedValue> = HashMap::new();
    subject_details.insert(
        "name".to_string(),
        Value::Str(sender.into()).try_to_owned().map_err(|e| {
            ComarError::Internal(format!("OwnedValue dönüşümü: {}", e))
        })?,
    );
    let subject = ("system-bus-name".to_string(), subject_details);

    // flags = 1 → AllowUserInteraction
    let flags: u32 = 1;

    match proxy
        .check_authorization(subject, &action_id_used, HashMap::new(), flags, "")
        .await
    {
        Ok((is_authorized, is_challenge, _details)) => {
            if is_authorized {
                Ok(PolicyResult::Yes)
            } else if is_challenge {
                Ok(PolicyResult::Challenge)
            } else {
                Ok(PolicyResult::No)
            }
        }
        Err(e) => {
            error!("PolicyKit sorgusu başarısız: {}", e);
            // C'deki davranış: hata durumunda POLICY_NO döndür, -1 değil
            Ok(PolicyResult::No)
        }
    }
}

// ── Gönderici UID sorgusu ─────────────────────────────────────────────────────

/// D-Bus sender'ın Unix UID'ini sorgular.
///
/// `dbus_bus_get_unix_user()` karşılığı.
async fn get_sender_uid(connection: &Connection, sender: &str) -> Result<u32> {
    // org.freedesktop.DBus.GetConnectionUnixUser(s bus_name) → u
    let dbus_proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(ComarError::Zbus)?;

    let bus_name = zbus::names::BusName::try_from(sender)
        .map_err(|e| ComarError::DBus(format!("Geçersiz bus adı: {}", e)))?;
    dbus_proxy
        .get_connection_unix_user(bus_name.as_ref())
        .await
        .map_err(|e| ComarError::DBus(e.to_string()))
}

// ── PolicyKit policy dosyaları ────────────────────────────────────────────────
// Mevcut C projesindeki 17 adet .policy dosyası data/polkit-1/actions/ altına
// kopyalanır. İçerikleri değişmez.

// ── Testler ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_result_variants() {
        assert_ne!(PolicyResult::Yes, PolicyResult::No);
        assert_ne!(PolicyResult::Yes, PolicyResult::Challenge);
        assert_ne!(PolicyResult::No,  PolicyResult::Challenge);
    }

    // NOT: PolicyKit D-Bus entegrasyon testleri gerçek system bus gerektirir.
    // Bunlar CI'da değil, kurulu bir LupuS sisteminde çalıştırılır.
    // Bkz. tests/ dizini altındaki entegrasyon test planı.
}
