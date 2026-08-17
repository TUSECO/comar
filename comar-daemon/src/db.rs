//! Model veritabanı — db.c karşılığı.
//!
//! XML model dosyalarını okuyup belleğe yükler.
//! Dosya yapısı değişmez; mevcut .xml modeller aynen kullanılır.

use crate::error::{ComarError, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, error, warn};

// ── Veri tipleri ──────────────────────────────────────────────────────────────

/// Method mi, signal mi?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberType {
    Method,
    Signal,
}

/// Tek bir method veya signal tanımı.
#[derive(Debug, Clone)]
pub struct MemberDef {
    pub kind:      MemberType,
    pub action_id: String,
    pub args_in:   Vec<String>,
    pub args_out:  Vec<String>,
    pub no_reply:  bool,
}

impl MemberDef {
    #[allow(dead_code)]
    pub fn signature_in(&self) -> String  { self.args_in.join("") }
    pub fn signature_out(&self) -> String {
        if self.no_reply { String::new() } else { self.args_out.join("") }
    }
}

pub type ModelDef      = HashMap<String, MemberDef>;
pub type ModelRegistry = HashMap<String, ModelDef>;

// ── Yardımcılar ───────────────────────────────────────────────────────────────

fn attr_value(e: &quick_xml::events::BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if a.key.as_ref() == name.as_bytes() {
            String::from_utf8(a.value.into_owned()).ok()
        } else {
            None
        }
    })
}

fn derive_action_id(
    e: &quick_xml::events::BytesStart,
    iface_name: &str,
    member_name: &str,
    interface_prefix: &str,
) -> String {
    if let Some(id) = attr_value(e, "action_id") {
        return id;
    }
    let alias = attr_value(e, "alias")
        .or_else(|| attr_value(e, "access_label"))
        .unwrap_or_else(|| member_name.to_string());
    format!("{}.{}.{}", interface_prefix, iface_name, alias).to_lowercase()
}

/// Bir method/signal için yeni MemberDef oluşturur.
fn make_member(
    e: &quick_xml::events::BytesStart,
    tag: &str,
    iface_name: &str,
    interface_prefix: &str,
    path: &Path,
) -> Result<(String, MemberDef)> {
    let name = attr_value(e, "name").ok_or_else(|| {
        ComarError::Xml(format!("{:?}: {} tag'inde name eksik", path, tag))
    })?;
    let action_id = derive_action_id(e, iface_name, &name, interface_prefix);
    let kind = if tag == "signal" { MemberType::Signal } else { MemberType::Method };
    Ok((name, MemberDef { kind, action_id, args_in: vec![], args_out: vec![], no_reply: false }))
}

/// Mevcut member'a arg ekler.
fn add_arg(member: &mut MemberDef, e: &quick_xml::events::BytesStart, path: &Path) {
    let typ = match attr_value(e, "type") {
        Some(t) => t,
        None => { warn!("arg tag'inde type yok: {:?}", path); return; }
    };
    let dir = attr_value(e, "direction").unwrap_or_else(|| "in".into());
    if member.kind == MemberType::Signal || dir == "out" {
        member.args_out.push(typ);
    } else {
        member.args_in.push(typ);
    }
}

// ── Ana yükleyici ─────────────────────────────────────────────────────────────

/// Tek bir .xml model dosyasını yükler. `db_validate_model` + `db_load_model` karşılığı.
pub fn load_model_file(
    path: &Path,
    registry: &mut ModelRegistry,
    interface_prefix: &str,
) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| ComarError::Xml(format!("Dosya okunamadı {:?}: {}", path, e)))?;

    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut current_iface: Option<String>           = None;
    let mut current_member: Option<(String, MemberDef)> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(ComarError::Xml(format!("{:?}: {}", path, e))),
            Ok(Event::Eof) => break,

            // Start: açık etiket <tag ...> — çocukları var
            Ok(Event::Start(ref e)) => {
                let tag_bytes = e.name();
                let tag = std::str::from_utf8(tag_bytes.as_ref()).unwrap_or("");
                match tag {
                    "comarModel" => {}
                    "interface" => {
                        let name = attr_value(e, "name").ok_or_else(||
                            ComarError::Xml(format!("{:?}: interface name yok", path)))?;
                        current_iface = Some(name.clone());
                        registry.entry(name).or_default();
                    }
                    "method" | "signal" => {
                        let iface = current_iface.as_deref().unwrap_or("");
                        let (name, member) = make_member(e, tag, iface, interface_prefix, path)?;
                        current_member = Some((name, member));
                    }
                    "arg" => {
                        if let Some((_, ref mut m)) = current_member { add_arg(m, e, path); }
                    }
                    "annotation" => {
                        if let Some((_, ref mut m)) = current_member {
                            let name = attr_value(e, "name");
                            let value = attr_value(e, "value");
                            if name.as_deref() == Some("org.freedesktop.DBus.Method.NoReply")
                                && value.as_deref() == Some("true")
                            {
                                m.no_reply = true;
                                m.args_out.clear();
                            }
                        }
                    }
                    other => { debug!("Bilinmeyen tag '{}' {:?}", other, path); }
                }
            }

            // Empty: self-closing etiket <tag .../> — çocukları YOK, End eventi de YOK
            // Örnek: <method name="updateAllRepositories"/>
            Ok(Event::Empty(ref e)) => {
                let tag_bytes = e.name();
                let tag = std::str::from_utf8(tag_bytes.as_ref()).unwrap_or("");
                match tag {
                    "interface" => {
                        let name = attr_value(e, "name").ok_or_else(||
                            ComarError::Xml(format!("{:?}: interface name yok", path)))?;
                        registry.entry(name).or_default();
                    }
                    "method" | "signal" => {
                        // Çocuğu yok → hemen kaydet
                        let iface = current_iface.as_deref().unwrap_or("");
                        let (name, member) = make_member(e, tag, iface, interface_prefix, path)?;
                        if let Some(model) = registry.get_mut(iface) {
                            model.insert(name, member);
                        }
                    }
                    "arg" => {
                        if let Some((_, ref mut m)) = current_member { add_arg(m, e, path); }
                    }
                    "annotation" => {
                        if let Some((_, ref mut m)) = current_member {
                            let name = attr_value(e, "name");
                            let value = attr_value(e, "value");
                            if name.as_deref() == Some("org.freedesktop.DBus.Method.NoReply")
                                && value.as_deref() == Some("true")
                            {
                                m.no_reply = true;
                                m.args_out.clear();
                            }
                        }
                    }
                    _ => {}
                }
            }

            // End: kapanış etiketi
            Ok(Event::End(ref e)) => {
                let _end_name = e.name();
                let tag = std::str::from_utf8(_end_name.as_ref()).unwrap_or("");
                match tag {
                    "method" | "signal" => {
                        if let (Some(iface), Some((name, member))) =
                            (&current_iface, current_member.take())
                        {
                            if let Some(model) = registry.get_mut(iface.as_str()) {
                                model.insert(name, member);
                            }
                        }
                    }
                    "interface" => { current_iface = None; }
                    _ => {}
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

/// Modeller dizinindeki tüm .xml dosyalarını yükler. `db_load_models()` karşılığı.
pub fn load_models(models_dir: &Path, interface_prefix: &str) -> Result<ModelRegistry> {
    if !models_dir.exists() {
        return Err(ComarError::Missing(format!(
            "Modeller dizini bulunamadı: {:?}", models_dir
        )));
    }
    let mut registry = ModelRegistry::new();
    for entry in std::fs::read_dir(models_dir).map_err(ComarError::Io)?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("xml") { continue; }
        debug!("Model yükleniyor: {:?}", path);
        load_model_file(&path, &mut registry, interface_prefix)
            .map_err(|e| { error!("{}", e); e })?;
    }
    if registry.is_empty() {
        return Err(ComarError::Missing("Hiç model bulunamadı.".into()));
    }
    debug!("Yüklenen model sayısı: {}", registry.len());
    Ok(registry)
}

/// `validate_model_member()` karşılığı.
pub fn validate_model_member(
    registry: &ModelRegistry, model: &str, member: &str, kind: &MemberType,
) -> bool {
    registry.get(model)
        .and_then(|m| m.get(member))
        .map(|def| &def.kind == kind)
        .unwrap_or(false)
}

/// `script_signature(model, method, 0)` karşılığı.
#[allow(dead_code)]
pub fn signature_in(registry: &ModelRegistry, model: &str, member: &str) -> Option<String> {
    registry.get(model)?.get(member).map(|d| d.signature_in())
}

/// `script_signature(model, method, 1)` karşılığı.
#[allow(dead_code)]
pub fn signature_out(registry: &ModelRegistry, model: &str, member: &str) -> Option<String> {
    registry.get(model)?.get(member).map(|d| d.signature_out())
}

// ── Testler ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = "tr.org.luppo.comar";

    fn parse(xml: &str) -> ModelRegistry {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.xml");
        std::fs::write(&path, xml).unwrap();
        let mut reg = ModelRegistry::new();
        load_model_file(&path, &mut reg, PREFIX).unwrap();
        reg
    }

    #[test]
    fn method_with_in_and_out_args() {
        let reg = parse(r#"<comarModel><interface name="User.Manager">
            <method name="addUser">
                <arg name="uid"  type="i" direction="in"/>
                <arg name="name" type="s" direction="in"/>
                <arg name="id"   type="i" direction="out"/>
            </method>
        </interface></comarModel>"#);
        let d = &reg["User.Manager"]["addUser"];
        assert_eq!(d.kind,         MemberType::Method);
        assert_eq!(d.args_in,      vec!["i","s"]);
        assert_eq!(d.args_out,     vec!["i"]);
        assert_eq!(d.signature_in(),  "is");
        assert_eq!(d.signature_out(), "i");
    }

    #[test]
    fn signal_args_always_out() {
        let reg = parse(r#"<comarModel><interface name="System.Service">
            <signal name="Changed">
                <arg name="service" type="s"/>
                <arg name="state"   type="s"/>
            </signal>
        </interface></comarModel>"#);
        let d = &reg["System.Service"]["Changed"];
        assert_eq!(d.kind, MemberType::Signal);
        assert!(d.args_in.is_empty());
        assert_eq!(d.args_out, vec!["s","s"]);
    }

    #[test]
    fn no_reply_clears_out_args() {
        let reg = parse(r#"<comarModel><interface name="System.Service">
            <method name="start" action_id="tr.org.luppo.comar.system.service.set">
                <annotation name="org.freedesktop.DBus.Method.NoReply" value="true"/>
            </method>
        </interface></comarModel>"#);
        let d = &reg["System.Service"]["start"];
        assert!(d.no_reply);
        assert!(d.args_out.is_empty());
        assert_eq!(d.signature_out(), "");
    }

    #[test]
    fn self_closing_method_registered() {
        let reg = parse(r#"<comarModel><interface name="System.Manager">
            <method name="updateAllRepositories"/>
            <method name="takeSnapshot"/>
        </interface></comarModel>"#);
        assert!(reg["System.Manager"].contains_key("updateAllRepositories"));
        assert!(reg["System.Manager"].contains_key("takeSnapshot"));
        let d = &reg["System.Manager"]["updateAllRepositories"];
        assert_eq!(d.kind, MemberType::Method);
        assert!(d.args_in.is_empty() && d.args_out.is_empty());
    }

    #[test]
    fn self_closing_with_access_label() {
        // Xorg.Display.xml: <method name="syncConfigs" access_label="set"/>
        let reg = parse(r#"<comarModel><interface name="Xorg.Display">
            <method name="syncConfigs" access_label="set"/>
        </interface></comarModel>"#);
        assert_eq!(
            reg["Xorg.Display"]["syncConfigs"].action_id,
            "tr.org.luppo.comar.xorg.display.set"
        );
    }

    #[test]
    fn explicit_action_id_preserved() {
        let reg = parse(r#"<comarModel><interface name="Net.Link">
            <method name="setState" action_id="tr.org.luppo.comar.net.link.setstate">
                <arg name="name"  type="s" direction="in"/>
                <arg name="state" type="s" direction="in"/>
                <annotation name="org.freedesktop.DBus.Method.NoReply" value="true"/>
            </method>
        </interface></comarModel>"#);
        let d = &reg["Net.Link"]["setState"];
        assert_eq!(d.action_id, "tr.org.luppo.comar.net.link.setstate");
        assert!(d.no_reply);
        assert_eq!(d.signature_in(), "ss");
    }

    #[test]
    fn empty_action_id_no_polkit() {
        let reg = parse(r#"<comarModel><interface name="Boot.Loader">
            <method name="listSystems" action_id="">
                <arg name="systems" type="a{s(sasas)}" direction="out"/>
            </method>
        </interface></comarModel>"#);
        assert_eq!(reg["Boot.Loader"]["listSystems"].action_id, "");
        assert_eq!(reg["Boot.Loader"]["listSystems"].signature_out(), "a{s(sasas)}");
    }

    #[test]
    fn access_label_derives_action_id() {
        let reg = parse(r#"<comarModel><interface name="User.Manager">
            <method name="userList" access_label="get"/>
        </interface></comarModel>"#);
        assert_eq!(
            reg["User.Manager"]["userList"].action_id,
            "tr.org.luppo.comar.user.manager.get"
        );
    }

    #[test]
    fn validate_member_kind() {
        let reg = parse(r#"<comarModel><interface name="Time.Clock">
            <method name="getDate" access_label="get">
                <arg name="date" type="s" direction="out"/>
            </method>
            <signal name="Changed"><arg name="item" type="s"/></signal>
        </interface></comarModel>"#);
        assert!( validate_model_member(&reg, "Time.Clock", "getDate", &MemberType::Method));
        assert!(!validate_model_member(&reg, "Time.Clock", "getDate", &MemberType::Signal));
        assert!( validate_model_member(&reg, "Time.Clock", "Changed", &MemberType::Signal));
        assert!(!validate_model_member(&reg, "Time.Clock", "missing", &MemberType::Method));
    }

    #[test]
    fn multiple_interfaces_in_one_file() {
        let reg = parse(r#"<comarModel>
            <interface name="Net.Link">
                <method name="linkInfo" action_id="">
                    <arg name="info" type="a{ss}" direction="out"/>
                </method>
            </interface>
            <interface name="Net.Stack">
                <method name="getHostName" action_id="">
                    <arg name="hostname" type="s" direction="out"/>
                </method>
            </interface>
        </comarModel>"#);
        assert!(reg.contains_key("Net.Link"));
        assert!(reg.contains_key("Net.Stack"));
        assert_eq!(reg["Net.Link"]["linkInfo"].signature_out(), "a{ss}");
    }

    #[test]
    fn load_models_from_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path1 = dir.path().join("model1.xml");
        let path2 = dir.path().join("model2.xml");
        let path3 = dir.path().join("ignored.txt");

        std::fs::write(&path1, r#"<comarModel><interface name="Model.One">
            <method name="m1"/>
        </interface></comarModel>"#).unwrap();
        std::fs::write(&path2, r#"<comarModel><interface name="Model.Two">
            <method name="m2"/>
        </interface></comarModel>"#).unwrap();
        std::fs::write(&path3, "not xml content").unwrap();

        let reg = load_models(dir.path(), PREFIX).unwrap();
        
        // Sadece .xml uzantılı 2 model yüklenmiş olmalı
        assert_eq!(reg.len(), 2);
        assert!(reg.contains_key("Model.One"));
        assert!(reg.contains_key("Model.Two"));
        assert!(reg["Model.One"].contains_key("m1"));
    }

    #[test]
    fn load_models_missing_dir_returns_error() {
        // Olmayan bir dizin hata döndürmeli
        let result = load_models(std::path::Path::new("/tmp/non-existent-comar-dir"), PREFIX);
        assert!(result.is_err());
    }

    #[test]
    fn load_models_empty_dir_returns_error() {
        // Boş dizin (model bulunamadı hatası) döndürmeli
        let dir = tempfile::tempdir().unwrap();
        let result = load_models(dir.path(), PREFIX);
        assert!(result.is_err());
    }

    #[test]
    fn load_real_model_files_integration() {
        let models_dir = std::path::PathBuf::from(
            "/home/claude/comar-src/comar/comar-master/models"
        );
        if !models_dir.exists() { return; } // CI'da kaynak yoksa atla

        let reg = load_models(&models_dir, PREFIX).unwrap();
        assert_eq!(reg.len(), 20);

        // updateAllRepositories self-closing
        assert!(reg["System.Manager"].contains_key("updateAllRepositories"));
        // start no_reply + action_id
        let start = &reg["System.Service"]["start"];
        assert!(start.no_reply);
        assert_eq!(start.action_id, "tr.org.luppo.comar.system.service.set");
        // listSystems boş action_id
        assert_eq!(reg["Boot.Loader"]["listSystems"].action_id, "");
    }
}
