use crate::error::{ComarError, Result};
use rust_i18n::t;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use zvariant::{OwnedValue, Value};
use kdl::KdlDocument;

#[derive(Debug, Deserialize)]
pub struct DeclarativeService {
    pub service: ServiceInfo,
    #[serde(default)]
    pub commands: HashMap<String, String>,
    #[serde(skip)]
    pub conf: Option<String>,
    #[serde(skip)]
    pub default: Option<String>,
    #[serde(skip)]
    pub notify_methods: HashSet<String>,
    #[serde(skip)]
    pub descriptions: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct ServiceInfo {
    #[serde(rename = "type", default = "default_service_type")]
    pub kind: String,
    #[serde(default)]
    pub description: String,
}

fn default_service_type() -> String {
    "script".to_string()
}

pub fn execute(
    path: &Path,
    method: &str,
    args: &[OwnedValue],
    lang: &str,
) -> Result<Vec<OwnedValue>> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        ComarError::Script(t!("error_io", locale = lang, error = e.to_string()).to_string())
    })?;

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let config: DeclarativeService = match ext {
        "json" => serde_json::from_str(&content).map_err(|e| {
            ComarError::Script(t!("error_xml", locale = lang, error = e.to_string()).to_string())
        })?,
        "kdl" => {
            let mut svc = parse_kdl(&content).map_err(|e| {
                ComarError::Script(t!("error_xml", locale = lang, error = e).to_string())
            })?;
            if !svc.descriptions.is_empty() {
                if let Some(d) = svc.descriptions.get(lang).or_else(|| svc.descriptions.get("en")) {
                    svc.service.description = d.clone();
                }
            }
            svc
        }
        _ => return Err(ComarError::Script(t!("error_missing_script", locale = lang, path = path.to_string_lossy()).to_string())),
    };

    let app = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let model = path.parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    let cmd_prefix = config.conf.as_ref().map(|conf_name| {
        format!(". /etc/conf.d/{} 2>/dev/null; ", conf_name)
    }).unwrap_or_default();

    match method {
        "info" => {
            let status = if let Some(cmd_str) = config.commands.get("status") {
                let full_cmd = format!("{}{}", cmd_prefix, cmd_str);
                if run_command(&full_cmd, &[]).is_ok() { "on" } else { "off" }
            } else {
                "unknown"
            };

            Ok(vec![
                OwnedValue::try_from(Value::from(config.service.kind)).unwrap(),
                OwnedValue::try_from(Value::from(config.service.description)).unwrap(),
                OwnedValue::try_from(Value::from(status)).unwrap(),
            ])
        }
        _ => {
            if let Some(cmd_str) = config.commands.get(method) {
                let full_cmd = format!("{}{}", cmd_prefix, cmd_str);
                run_command(&full_cmd, args)?;

                if config.notify_methods.contains(method) {
                    let state = match method {
                        "start" => "started",
                        "stop" => "stopped",
                        "reload" => "started",
                        _ => method,
                    };
                    crate::signal_bus::send_signal(crate::signal_bus::SignalRequest {
                        path: format!("/package/{}", app),
                        model: model.clone(),
                        signal: "Changed".to_string(),
                        args: vec![
                            OwnedValue::try_from(Value::from(app)).unwrap(),
                            OwnedValue::try_from(Value::from(state)).unwrap(),
                        ],
                    });
                }

                Ok(vec![])
            } else {
                Err(ComarError::Missing(t!(
                    "error_missing_method",
                    locale = lang,
                    method = method,
                    path = path.to_string_lossy()
                ).to_string()))
            }
        }
    }
}

fn parse_kdl(content: &str) -> std::result::Result<DeclarativeService, String> {
    let doc: KdlDocument = content.parse().map_err(|e: kdl::KdlError| e.to_string())?;

    if let Some(package_node) = doc.get("package") {
        let mut commands = HashMap::new();
        let mut description = String::new();
        if let Some(children) = package_node.children() {
            for child in children.nodes() {
                let name = child.name().value();
                match name {
                    "description" => {
                        if let Some(val) = child.get(0).and_then(|v| v.as_string()) {
                            description = val.to_string();
                        }
                    }
                    _ => {
                        if let Some(val) = child.get(0).and_then(|v| v.as_string()) {
                            commands.insert(name.to_string(), val.to_string());
                            let camel_case = name.split('-')
                                .enumerate()
                                .map(|(i, s)| {
                                    if i == 0 {
                                        s.to_string()
                                    } else {
                                        let mut c = s.chars();
                                        match c.next() {
                                            None => String::new(),
                                            Some(f) => f.to_uppercase().collect::<String>() + c.as_str()
                                        }
                                    }
                                })
                                .collect::<String>();
                            if camel_case != name {
                                commands.insert(camel_case, val.to_string());
                            }
                        }
                    }
                }
            }
        }
        return Ok(DeclarativeService {
            service: ServiceInfo {
                kind: "package".to_string(),
                description,
            },
            commands,
            conf: None,
            default: None,
            notify_methods: HashSet::new(),
            descriptions: HashMap::new(),
        });
    }

    if let Some(service_node) = doc.get("service") {
        let mut commands = HashMap::new();
        let mut kind = "script".to_string();
        let mut description = String::new();
        let mut conf = None;
        let mut default = None;
        let mut notify_methods = HashSet::new();
        let mut descriptions = HashMap::new();

        if let Some(children) = service_node.children() {
            for child in children.nodes() {
                let name = child.name().value();
                match name {
                    "type" => {
                        if let Some(val) = child.get(0).and_then(|v| v.as_string()) {
                            kind = val.to_string();
                        }
                    }
                    "description" => {
                        if let Some(nodes) = child.children() {
                            for node in nodes.nodes() {
                                let locale = node.name().value();
                                if let Some(val) = node.get(0).and_then(|v| v.as_string()) {
                                    descriptions.insert(locale.to_string(), val.to_string());
                                }
                            }
                        } else if let Some(val) = child.get(0).and_then(|v| v.as_string()) {
                            description = val.to_string();
                        }
                    }
                    "conf" => {
                        if let Some(val) = child.get(0).and_then(|v| v.as_string()) {
                            conf = Some(val.to_string());
                        }
                    }
                    "default" => {
                        if let Some(val) = child.get(0).and_then(|v| v.as_string()) {
                            default = Some(val.to_string());
                        }
                    }
                    _ => {
                        if let Some(val) = child.get(0).and_then(|v| v.as_string()) {
                            let donotify = child.get("donotify")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            commands.insert(name.to_string(), val.to_string());
                            if donotify {
                                notify_methods.insert(name.to_string());
                            }
                        }
                    }
                }
            }
        }
        return Ok(DeclarativeService {
            service: ServiceInfo { kind, description },
            commands,
            conf,
            default,
            notify_methods,
            descriptions,
        });
    }

    Err("No 'package' or 'service' node found in KDL".to_string())
}

fn run_command(cmd_str: &str, args: &[OwnedValue]) -> Result<()> {
    let mut child = Command::new("sh");
    child.arg("-c").arg(cmd_str).arg("sh"); // "sh" becomes $0
    for arg in args {
        use std::ops::Deref;
        let val_str = match arg.deref() {
            Value::Str(s) => s.as_str().to_string(),
            _ => String::new(),
        };
        child.arg(val_str);
    }

    let status = child.status().map_err(|e| {
        ComarError::Script(format!("Komut çalıştırılamadı '{}': {}", cmd_str, e))
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(ComarError::Script(format!(
            "Komut başarısız oldu (exit code: {:?}): {}",
            status.code(),
            cmd_str
        )))
    }
}
