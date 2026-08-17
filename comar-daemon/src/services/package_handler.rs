use crate::error::{ComarError, Result};
use kdl::KdlDocument;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;
use zvariant::{OwnedValue, Value};

// ── Veri yapıları ─────────────────────────────────────────────────────

#[derive(Debug)]
pub struct HandlerAction {
    pub command: Vec<String>,
    pub match_rules: Vec<MatchRule>,
    pub exclude_rules: Vec<MatchRule>,
    pub transforms: Vec<PathTransform>,
}

#[derive(Debug)]
pub enum MatchRule {
    EndsWith(String),
    StartsWith(String),
}

#[derive(Debug)]
pub struct PathTransform {
    pub from: String,
    pub to: String,
}

#[derive(Debug)]
struct HandlerDef {
    name: String,
    description: String,
    setup: Vec<HandlerAction>,
    cleanup: Vec<HandlerAction>,
    post_cleanup: Vec<HandlerAction>,
}

// ── Global handler kaydı ──────────────────────────────────────────────

static HANDLERS: LazyLock<HashMap<String, HandlerDef>> = LazyLock::new(load_all_handlers);

fn handler_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = vec![];
    if let Ok(cwd) = std::env::current_dir() {
        let test = cwd.join("comar").join("handlers");
        if test.is_dir() {
            dirs.push(test);
        }
    }
    let sys = std::path::PathBuf::from("/usr/share/comar/handlers");
    if sys.is_dir() {
        dirs.push(sys);
    }
    dirs
}

fn load_all_handlers() -> HashMap<String, HandlerDef> {
    let mut handlers = HashMap::new();
    for dir in handler_dirs() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |e| e != "kdl") {
                continue;
            }
            match load_handler(&path) {
                Ok(h) => {
                    handlers.insert(h.name.clone(), h);
                }
                Err(e) => {
                    tracing::warn!("PackageHandler yüklenemedi {:?}: {}", path, e);
                }
            }
        }
    }
    handlers
}

fn load_handler(path: &Path) -> Result<HandlerDef> {
    let content = std::fs::read_to_string(path).map_err(ComarError::Io)?;
    let doc: KdlDocument = content
        .parse()
        .map_err(|e: kdl::KdlError| ComarError::Script(e.to_string()))?;

    let root = doc
        .get("packagehandler")
        .ok_or_else(|| ComarError::Script("No 'packagehandler' node".into()))?;

    let name = root
        .get(0)
        .and_then(|v| v.as_string())
        .map(|s| s.to_string())
        .ok_or_else(|| ComarError::Script("Missing handler name".into()))?;

    let mut description = String::new();
    let mut setup = vec![];
    let mut cleanup = vec![];
    let mut post_cleanup = vec![];

    if let Some(children) = root.children() {
        for child in children.nodes() {
            let cname = child.name().value();
            match cname {
                "description" => {
                    if let Some(val) = child.get(0).and_then(|v| v.as_string()) {
                        description = val.to_string();
                    }
                }
                "setup" => {
                    let action = parse_action(child);
                    if !action.command.is_empty() {
                        setup.push(action);
                    }
                }
                "cleanup" => {
                    let action = parse_action(child);
                    if !action.command.is_empty() {
                        cleanup.push(action);
                    }
                }
                "post-cleanup" => {
                    let action = parse_action(child);
                    if !action.command.is_empty() {
                        post_cleanup.push(action);
                    }
                }
                _ => {}
            }
        }
    }

    Ok(HandlerDef {
        name,
        description,
        setup,
        cleanup,
        post_cleanup,
    })
}

fn parse_action(node: &kdl::KdlNode) -> HandlerAction {
    let mut command = vec![];
    let mut match_rules = vec![];
    let mut exclude_rules = vec![];
    let mut transforms = vec![];

    if let Some(children) = node.children() {
        for child in children.nodes() {
            let cname = child.name().value();
            match cname {
                "command" => {
                    for entry in child.entries() {
                        if let Some(s) = entry.value().as_string() {
                            command.push(s.to_string());
                        }
                    }
                }
                "match" => {
                    parse_rules(child, &mut match_rules);
                }
                "exclude" => {
                    parse_rules(child, &mut exclude_rules);
                }
                "transform" => {
                    if let Some(tc) = child.children() {
                        for t in tc.nodes() {
                            if let Some(from) = t.get(0).and_then(|v| v.as_string()) {
                                if let Some(to) = t.get(1).and_then(|v| v.as_string()) {
                                    transforms.push(PathTransform {
                                        from: from.to_string(),
                                        to: to.to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    HandlerAction {
        command,
        match_rules,
        exclude_rules,
        transforms,
    }
}

fn parse_rules(node: &kdl::KdlNode, rules: &mut Vec<MatchRule>) {
    if let Some(children) = node.children() {
        for child in children.nodes() {
            let modifier = child.name().value();
            if let Some(val) = child.get(0).and_then(|v| v.as_string()) {
                match modifier {
                    "ends-with" => rules.push(MatchRule::EndsWith(val.to_string())),
                    "starts-with" => rules.push(MatchRule::StartsWith(val.to_string())),
                    _ => {}
                }
            }
        }
    }
}

// ── Paket dosya listesi XML ayrıştırma ─────────────────────────────────

fn extract_paths(filepath: &str) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(filepath).map_err(ComarError::Io)?;
    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(true);
    let mut paths = vec![];
    let mut in_path = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == b"Path" {
                    in_path = true;
                }
            }
            Ok(Event::Text(ref e)) => {
                if in_path {
                    if let Ok(text) = e.unescape() {
                        paths.push(text.to_string());
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name().as_ref() == b"Path" {
                    in_path = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(ComarError::Script(format!("XML parse error: {}", e)));
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(paths)
}

fn match_paths(paths: &[String], action: &HandlerAction) -> Vec<String> {
    paths
        .iter()
        .filter(|p| {
            let matches = action.match_rules.is_empty()
                || action.match_rules.iter().all(|r| match r {
                    MatchRule::EndsWith(s) => p.ends_with(s),
                    MatchRule::StartsWith(s) => p.starts_with(s),
                });
            let excluded = action.exclude_rules.iter().any(|r| match r {
                MatchRule::EndsWith(s) => p.ends_with(s),
                MatchRule::StartsWith(s) => p.starts_with(s),
            });
            matches && !excluded
        })
        .cloned()
        .collect()
}

fn transform_paths(paths: &[String], transforms: &[PathTransform]) -> Vec<String> {
    if transforms.is_empty() {
        return paths.to_vec();
    }
    let mut result = vec![];
    for p in paths {
        let mut added = false;
        for t in transforms {
            if let Some(rest) = p.strip_suffix(&t.from) {
                result.push(format!("{}{}", rest, t.to));
                added = true;
            }
        }
        if !added {
            result.push(p.clone());
        }
    }
    result
}

// ── Komut çalıştırma ───────────────────────────────────────────────────

fn run_action(action: &HandlerAction, metapath: &str, filepath: &str) -> Result<()> {
    if action.command.is_empty() {
        return Ok(());
    }

    let all_paths = extract_paths(filepath)?;
    let matched = match_paths(&all_paths, action);
    let expanded = transform_paths(&matched, &action.transforms);

    if !action.match_rules.is_empty() && matched.is_empty() {
        return Ok(());
    }

    let mut cmd = Command::new(&action.command[0]);
    for arg in &action.command[1..] {
        cmd.arg(arg);
    }
    for path in &expanded {
        cmd.arg(format!("/{}", path));
    }
    cmd.env("METAPATH", metapath);
    cmd.env("FILEPATH", filepath);

    let status = cmd.status().map_err(|e| {
        ComarError::Script(format!("Handler command failed: {}", e))
    })?;

    if !status.success() {
        tracing::warn!(
            "Handler exited with code {:?}: {:?}",
            status.code(),
            action.command
        );
    }

    Ok(())
}

// ── Dışa açık API ──────────────────────────────────────────────────────

pub fn run_all(method: &str, args: &[OwnedValue]) -> Result<Vec<OwnedValue>> {
    let metapath = args
        .first()
        .and_then(|v| {
            use std::ops::Deref;
            match v.deref() {
                Value::Str(s) => Some(s.as_str().to_string()),
                _ => None,
            }
        })
        .unwrap_or_default();
    let filepath = args
        .get(1)
        .and_then(|v| {
            use std::ops::Deref;
            match v.deref() {
                Value::Str(s) => Some(s.as_str().to_string()),
                _ => None,
            }
        })
        .unwrap_or_default();

    let handlers = &*HANDLERS;
    match method {
        "setupPackage" | "cleanupPackage" | "postCleanupPackage" => {
            for handler in handlers.values() {
                let actions = match method {
                    "setupPackage" => &handler.setup,
                    "cleanupPackage" => &handler.cleanup,
                    "postCleanupPackage" => &handler.post_cleanup,
                    _ => unreachable!(),
                };
                for action in actions {
                    if let Err(e) = run_action(action, &metapath, &filepath) {
                        tracing::warn!("Handler '{}' {} failed: {}", handler.name, method, e);
                    }
                }
            }
        }
        _ => {
            return Err(ComarError::Missing(format!(
                "System.PackageHandler: unknown method '{}'",
                method
            )))
        }
    }

    Ok(vec![])
}

#[allow(dead_code)]
pub fn list_handlers() -> Vec<(String, String)> {
    let handlers = &*HANDLERS;
    handlers
        .values()
        .map(|h| (h.name.clone(), h.description.clone()))
        .collect()
}

// ── Testler ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_extract_paths() {
        let xml = "<Files>
            <File><Path>usr/lib/python3.11/site.py</Path></File>
            <File><Path>usr/share/doc/python/readme</Path></File>
        </Files>";

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.xml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(xml.as_bytes()).unwrap();
        drop(f);

        let paths = extract_paths(path.to_str().unwrap()).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&"usr/lib/python3.11/site.py".to_string()));
    }

    #[test]
    fn test_match_paths_ends_with() {
        let paths = vec![
            "usr/lib/a.py".into(),
            "usr/share/doc/a.py".into(),
            "usr/lib/a.txt".into(),
        ];
        let action = HandlerAction {
            command: vec![],
            match_rules: vec![MatchRule::EndsWith(".py".into())],
            exclude_rules: vec![],
            transforms: vec![],
        };
        let matched = match_paths(&paths, &action);
        assert_eq!(matched.len(), 2);
        assert!(matched.contains(&"usr/lib/a.py".to_string()));
    }

    #[test]
    fn test_match_paths_with_exclude() {
        let paths = vec![
            "usr/lib/a.py".into(),
            "usr/share/doc/a.py".into(),
        ];
        let action = HandlerAction {
            command: vec![],
            match_rules: vec![MatchRule::EndsWith(".py".into())],
            exclude_rules: vec![MatchRule::StartsWith("usr/share/doc/".into())],
            transforms: vec![],
        };
        let matched = match_paths(&paths, &action);
        assert_eq!(matched.len(), 1);
        assert!(matched.contains(&"usr/lib/a.py".to_string()));
    }

    #[test]
    fn test_transform_paths() {
        let paths = vec!["usr/lib/a.py".into()];
        let transforms = vec![
            PathTransform {
                from: ".py".into(),
                to: ".pyc".into(),
            },
            PathTransform {
                from: ".py".into(),
                to: ".pyo".into(),
            },
        ];
        let expanded = transform_paths(&paths, &transforms);
        assert_eq!(expanded.len(), 2);
        assert!(expanded.contains(&"usr/lib/a.pyc".to_string()));
        assert!(expanded.contains(&"usr/lib/a.pyo".to_string()));
    }
}
