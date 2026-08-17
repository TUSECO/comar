use crate::error::{ComarError, Result};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use zvariant::{OwnedValue, Value};



pub struct UserManager;

impl super::ServiceHandler for UserManager {
    fn handle(
        &self,
        method: &str,
        args: &[OwnedValue],
        _lang: &str,
    ) -> Result<Vec<OwnedValue>> {
        match method {
            "userList" => user_list(),
            "userInfo" => user_info(args),
            "addUser" => add_user(args),
            "setUser" => set_user(args),
            "deleteUser" => delete_user(args),
            "groupList" => group_list(),
            "addGroup" => add_group(args),
            "deleteGroup" => delete_group(args),
            "grantAuthorization" => grant_authorization(args),
            "revokeAuthorization" => revoke_authorization(args),
            "blockAuthorization" => block_authorization(args),
            "listUserAuthorizations" => list_user_authorizations(args),
            _ => Err(ComarError::Missing(format!(
                "User.Manager: unknown method '{}'",
                method
            ))),
        }
    }
}

// ── Veri yapıları ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct User {
    name: String,
    uid: u32,
    gid: u32,
    realname: String,
    homedir: String,
    shell: String,
    password: String,
    pwrest: Vec<String>,
}

#[derive(Debug, Clone)]
struct Group {
    name: String,
    gid: u32,
    members: Vec<String>,
}

// ── Veritabanı ─────────────────────────────────────────────────────────

struct Database {
    users: HashMap<u32, User>,
    groups: HashMap<u32, Group>,
}

impl Database {
    fn read() -> Result<Self> {
        let users = read_passwd()?;
        let groups = read_group()?;
        Ok(Database { users, groups })
    }

    fn sync(&self) -> Result<()> {
        let _lock = PwdLock::lock();
        write_passwd(&self.users)?;
        write_shadow(&self.users)?;
        write_group(&self.groups)?;
        Ok(())
    }

    fn set_groups(&mut self, name: &str, grouplist: &[String]) {
        for g in self.groups.values_mut() {
            if g.members.contains(&name.to_string()) && !grouplist.contains(&g.name) {
                g.members.retain(|m| m != name);
            } else if grouplist.contains(&g.name) && !g.members.contains(&name.to_string()) {
                g.members.push(name.to_string());
            }
        }
    }
}

// ── Dosya kilidi ───────────────────────────────────────────────────────

struct PwdLock;

impl PwdLock {
    fn lock() -> Self {
        let lockpath = "/etc/.pwd.lock";
        loop {
            if let Ok(_f) = fs::File::create(lockpath) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        PwdLock
    }
}

impl Drop for PwdLock {
    fn drop(&mut self) {
        let _ = fs::remove_file("/etc/.pwd.lock");
    }
}

// ── /etc/passwd okuma/yazma ────────────────────────────────────────────

fn read_passwd() -> Result<HashMap<u32, User>> {
    let content = fs::read_to_string("/etc/passwd")
        .map_err(|e| ComarError::Io(e))?;
    let mut users = HashMap::new();
    for line in content.lines() {
        if line.is_empty() { continue; }
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 7 {
            if let Ok(uid) = parts[2].parse::<u32>() {
                users.insert(uid, User {
                    name: parts[0].to_string(),
                    uid,
                    gid: parts[3].parse().unwrap_or(0),
                    realname: parts[4].to_string(),
                    homedir: parts[5].to_string(),
                    shell: parts[6].to_string(),
                    password: String::new(),
                    pwrest: vec![],
                });
            }
        }
    }
    let shadow = fs::read_to_string("/etc/shadow").ok();
    if let Some(shadow) = shadow {
        for line in shadow.lines() {
            if line.is_empty() { continue; }
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 {
                if let Some(user) = users.values_mut().find(|u| u.name == parts[0]) {
                    user.password = parts[1].to_string();
                    user.pwrest = parts[2..].iter().map(|s| s.to_string()).collect();
                }
            }
        }
    }
    Ok(users)
}

fn write_passwd(users: &HashMap<u32, User>) -> Result<()> {
    let mut out = String::new();
    let mut keys: Vec<&u32> = users.keys().collect();
    keys.sort();
    for uid in keys {
        if let Some(u) = users.get(uid) {
            out.push_str(&format!(
                "{}:x:{}:{}:{}:{}:{}\n",
                u.name, u.uid, u.gid, u.realname, u.homedir, u.shell
            ));
        }
    }
    write_file_atomic("/etc/passwd", &out)
}

fn write_shadow(users: &HashMap<u32, User>) -> Result<()> {
    let mut out = String::new();
    let mut keys: Vec<&u32> = users.keys().collect();
    keys.sort();
    for uid in keys {
        if let Some(u) = users.get(uid) {
            let pw = if u.password.is_empty() { "*".to_string() } else { u.password.clone() };
            let rest = if u.pwrest.is_empty() {
                "13094:0:99999:7:::".to_string()
            } else {
                u.pwrest.join(":")
            };
            out.push_str(&format!("{}:{}:{}\n", u.name, pw, rest));
        }
    }
    write_file_atomic("/etc/shadow", &out)
}

// ── /etc/group okuma/yazma ─────────────────────────────────────────────

fn read_group() -> Result<HashMap<u32, Group>> {
    let content = fs::read_to_string("/etc/group")
        .map_err(|e| ComarError::Io(e))?;
    let mut groups = HashMap::new();
    for line in content.lines() {
        if line.is_empty() { continue; }
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 {
            if let Ok(gid) = parts[2].parse::<u32>() {
                let members: Vec<String> = parts.get(3)
                    .map(|m| m.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect())
                    .unwrap_or_default();
                groups.insert(gid, Group {
                    name: parts[0].to_string(),
                    gid,
                    members,
                });
            }
        }
    }
    Ok(groups)
}

fn write_group(groups: &HashMap<u32, Group>) -> Result<()> {
    let mut out = String::new();
    let mut keys: Vec<&u32> = groups.keys().collect();
    keys.sort();
    for gid in keys {
        if let Some(g) = groups.get(gid) {
            out.push_str(&format!(
                "{}:x:{}:{}\n",
                g.name, g.gid, g.members.join(",")
            ));
        }
    }
    write_file_atomic("/etc/group", &out)
}

fn write_file_atomic(path: &str, content: &str) -> Result<()> {
    let tmp = format!("{}.tmp", path);
    fs::write(&tmp, content.as_bytes()).map_err(|e| ComarError::Io(e))?;
    fs::rename(&tmp, path).map_err(|e| ComarError::Io(e))
}

// ── Parola hash'leme (MD5 crypt) ───────────────────────────────────────

fn shadow_crypt(password: &str) -> String {
    let output = Command::new("openssl")
        .arg("passwd")
        .arg("-1")
        .arg(password)
        .output();
    match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(_) => "*".to_string(),
    }
}

// ── Yardımcılar ────────────────────────────────────────────────────────

fn arg_to_u32(args: &[OwnedValue], idx: usize) -> Option<u32> {
    args.get(idx).and_then(|v| {
        use std::ops::Deref;
        match v.deref() {
            Value::I32(n) if *n >= 0 => Some(*n as u32),
            Value::U32(n) => Some(*n),
            Value::I64(n) if *n >= 0 && *n <= u32::MAX as i64 => Some(*n as u32),
            _ => None,
        }
    })
}

fn arg_to_string_opt(args: &[OwnedValue], idx: usize) -> Option<String> {
    args.get(idx).and_then(|v| {
        use std::ops::Deref;
        match v.deref() {
            Value::Str(s) => Some(s.as_str().to_string()),
            _ => None,
        }
    })
}

fn arg_to_string_array(args: &[OwnedValue], idx: usize) -> Vec<String> {
    args.get(idx)
        .and_then(|v| {
            use std::ops::Deref;
            match v.deref() {
                Value::Array(arr) => Some(
                    arr.iter()
                        .filter_map(|e| match e {
                            Value::Str(s) => Some(s.as_str().to_string()),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            }
        })
        .unwrap_or_default()
}

fn arg_to_bool(args: &[OwnedValue], idx: usize) -> bool {
    args.get(idx).map_or(false, |v| {
        use std::ops::Deref;
        match v.deref() {
            Value::Bool(b) => *b,
            Value::I32(n) => *n != 0,
            _ => false,
        }
    })
}

// ── Metodlar ────────────────────────────────────────────────────────────

fn user_list() -> Result<Vec<OwnedValue>> {
    let db = Database::read()?;
    let list: Vec<OwnedValue> = db
        .users
        .iter()
        .map(|(_, u)| {
            let mut sb = zvariant::StructureBuilder::new();
            sb.push_value(Value::I32(u.uid as i32));
            sb.push_value(Value::from(u.name.clone()));
            sb.push_value(Value::from(u.realname.clone()));
            OwnedValue::try_from(Value::Structure(sb.build())).unwrap()
        })
        .collect();
    Ok(vec![OwnedValue::try_from(Value::Array(
        zvariant::Array::from(list),
    ))
    .unwrap()])
}

fn user_info(args: &[OwnedValue]) -> Result<Vec<OwnedValue>> {
    let uid = arg_to_u32(args, 0).unwrap_or(u32::MAX);
    let db = Database::read()?;
    match db.users.get(&uid) {
        Some(u) => {
            let mut groups: Vec<String> = db
                .groups
                .values()
                .filter(|g| g.members.contains(&u.name))
                .map(|g| g.name.clone())
                .collect();
            if let Some(primary) = db.groups.get(&u.gid) {
                groups.retain(|g| g != &primary.name);
                groups.insert(0, primary.name.clone());
            }
            Ok(vec![
                OwnedValue::try_from(Value::from(u.name.clone())).unwrap(),
                OwnedValue::try_from(Value::from(u.realname.clone())).unwrap(),
                OwnedValue::try_from(Value::from(u.gid as i32)).unwrap(),
                OwnedValue::try_from(Value::from(u.homedir.clone())).unwrap(),
                OwnedValue::try_from(Value::from(u.shell.clone())).unwrap(),
                OwnedValue::try_from(Value::from(groups)).unwrap(),
            ])
        }
        None => Err(ComarError::Invalid("No user with given ID".to_string())),
    }
}

fn add_user(args: &[OwnedValue]) -> Result<Vec<OwnedValue>> {
    let uid_in: i32 = arg_to_u32(args, 0).map(|v| v as i32).unwrap_or(-1);
    let name = arg_to_string_opt(args, 1).ok_or_else(|| ComarError::Invalid("Missing user name".into()))?;
    let realname = arg_to_string_opt(args, 2).unwrap_or_default();
    let homedir = arg_to_string_opt(args, 3).unwrap_or_else(|| format!("/home/{}", name));
    let shell = arg_to_string_opt(args, 4).unwrap_or_else(|| "/bin/bash".to_string());
    let password = arg_to_string_opt(args, 5).unwrap_or_default();
    let groups = arg_to_string_array(args, 6);
    let grants = arg_to_string_array(args, 7);
    let blocks = arg_to_string_array(args, 8);

    if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Err(ComarError::Invalid("User name is invalid.".into()));
    }
    if name.chars().any(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '-') {
        return Err(ComarError::Invalid("User name is invalid.".into()));
    }

    let mut db = Database::read()?;

    let uid = if uid_in == -1 {
        (1000..65000).find(|i| !db.users.contains_key(i)).unwrap_or(65534)
    } else {
        let uid = uid_in as u32;
        if db.users.contains_key(&uid) {
            return Err(ComarError::Invalid("This user ID is already used.".into()));
        }
        uid
    };

    if db.users.values().any(|u| u.name == name) {
        return Err(ComarError::Invalid("This user name is already used.".into()));
    }

    let groups = if groups.is_empty() {
        vec!["nogroup".to_string()]
    } else {
        groups
    };
    for g in &groups {
        if !db.groups.values().any(|grp| grp.name == *g) {
            return Err(ComarError::Invalid(format!("No such group exists: {}", g)));
        }
    }

    let gid = db.groups.values().find(|g| g.name == groups[0]).map(|g| g.gid).unwrap_or(100);

    let pw = if password.is_empty() {
        "*".to_string()
    } else {
        shadow_crypt(&password)
    };

    let user = User {
        name: name.clone(),
        uid,
        gid,
        realname,
        homedir: homedir.clone(),
        shell,
        password: pw,
        pwrest: vec!["13094".into(), "0".into(), "99999".into(), "7".into(), String::new(), String::new(), String::new()],
    };

    db.users.insert(uid, user);
    db.set_groups(&name, &groups);
    db.sync()?;

    let uid_val = uid as u32;
    if uid >= 1000 || homedir.starts_with("/home/") {
        setup_home(uid, gid, &homedir)?;
    }

    for g in &grants {
        let _ = grant_polkit(uid, g);
    }
    for b in &blocks {
        let _ = block_polkit(uid, b);
    }

    Ok(vec![OwnedValue::try_from(Value::from(uid_val)).unwrap()])
}

fn set_user(args: &[OwnedValue]) -> Result<Vec<OwnedValue>> {
    let uid = arg_to_u32(args, 0).unwrap_or(u32::MAX);
    let realname = arg_to_string_opt(args, 1).unwrap_or_default();
    let homedir = arg_to_string_opt(args, 2).unwrap_or_default();
    let shell = arg_to_string_opt(args, 3).unwrap_or_default();
    let password = arg_to_string_opt(args, 4).unwrap_or_default();
    let groups = arg_to_string_array(args, 5);

    let mut db = Database::read()?;

    if !db.users.contains_key(&uid) {
        return Err(ComarError::Invalid("No user with given ID".into()));
    }
    for g in &groups {
        if !db.groups.values().any(|grp| grp.name == *g) {
            return Err(ComarError::Invalid(format!("No such group: {}", g)));
        }
    }

    let user_name = db.users[&uid].name.clone();

    if let Some(u) = db.users.get_mut(&uid) {
        if !realname.is_empty() { u.realname = realname; }
        if !homedir.is_empty() { u.homedir = homedir; }
        if !shell.is_empty() { u.shell = shell; }
        if !password.is_empty() { u.password = shadow_crypt(&password); }
    }

    if !groups.is_empty() {
        db.set_groups(&user_name, &groups);
    }
    db.sync()?;
    Ok(vec![])
}

fn delete_user(args: &[OwnedValue]) -> Result<Vec<OwnedValue>> {
    let uid = arg_to_u32(args, 0).unwrap_or(u32::MAX);
    let deletefiles = arg_to_bool(args, 1);

    if uid == 0 {
        return Err(ComarError::Invalid("You cant delete root user.".into()));
    }

    let mut db = Database::read()?;
    match db.users.remove(&uid) {
        Some(u) => {
            let home = u.homedir.clone();
            db.set_groups(&u.name, &[]);
            let _ = revoke_polkit_all(uid);
            db.sync()?;
            if deletefiles {
                let _ = Command::new("rm").arg("-rf").arg(&home).output();
            }
            Ok(vec![])
        }
        None => Err(ComarError::Invalid("No user with given ID".into())),
    }
}

fn setup_home(uid: u32, gid: u32, path: &str) -> Result<()> {
    let p = Path::new(path);
    if !p.exists() {
        let _ = Command::new("cp")
            .arg("-r")
            .arg("/etc/skel")
            .arg(path)
            .output();
    }
    let _ = Command::new("chown")
        .arg("-R")
        .arg(format!("{}:{}", uid, gid))
        .arg(path)
        .output();
            fs::set_permissions(p, std::fs::Permissions::from_mode(0o711)).ok();
    Ok(())
}

fn group_list() -> Result<Vec<OwnedValue>> {
    let db = Database::read()?;
    let list: Vec<OwnedValue> = db
        .groups
        .iter()
        .map(|(_, g)| {
            let mut sb = zvariant::StructureBuilder::new();
            sb.push_value(Value::I32(g.gid as i32));
            sb.push_value(Value::from(g.name.clone()));
            OwnedValue::try_from(Value::Structure(sb.build())).unwrap()
        })
        .collect();
    Ok(vec![OwnedValue::try_from(Value::Array(
        zvariant::Array::from(list),
    ))
    .unwrap()])
}

fn add_group(args: &[OwnedValue]) -> Result<Vec<OwnedValue>> {
    let gid_in: i32 = arg_to_u32(args, 0).map(|v| v as i32).unwrap_or(-1);
    let name = arg_to_string_opt(args, 1).ok_or_else(|| ComarError::Invalid("Missing group name".into()))?;

    if name.is_empty() || name.chars().any(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '-') {
        return Err(ComarError::Invalid(format!("Invalid group name: {}", name)));
    }

    let mut db = Database::read()?;

    if db.groups.values().any(|g| g.name == name) {
        return Err(ComarError::Invalid("This group name is already used.".into()));
    }

    let gid = if gid_in == -1 {
        (1000..65000).find(|i| !db.groups.contains_key(i)).unwrap_or(65534)
    } else {
        let gid = gid_in as u32;
        if db.groups.contains_key(&gid) {
            return Err(ComarError::Invalid("This group ID is already used.".into()));
        }
        gid
    };

    db.groups.insert(gid, Group {
        name: name.clone(),
        gid,
        members: vec![],
    });
    db.sync()?;

    Ok(vec![OwnedValue::try_from(Value::from(gid)).unwrap()])
}

fn delete_group(args: &[OwnedValue]) -> Result<Vec<OwnedValue>> {
    let gid = arg_to_u32(args, 0).unwrap_or(u32::MAX);
    let mut db = Database::read()?;
    db.groups.remove(&gid);
    db.sync()?;
    Ok(vec![])
}

fn grant_authorization(args: &[OwnedValue]) -> Result<Vec<OwnedValue>> {
    let uid = arg_to_u32(args, 0).unwrap_or(u32::MAX);
    let action = arg_to_string_opt(args, 1).unwrap_or_default();
    Ok(vec![OwnedValue::try_from(Value::Bool(grant_polkit(uid, &action))).unwrap()])
}

fn revoke_authorization(args: &[OwnedValue]) -> Result<Vec<OwnedValue>> {
    let uid = arg_to_u32(args, 0).unwrap_or(u32::MAX);
    let action = arg_to_string_opt(args, 1).unwrap_or_default();
    if action == "*" {
        Ok(vec![OwnedValue::try_from(Value::Bool(revoke_polkit_all(uid))).unwrap()])
    } else {
        Ok(vec![OwnedValue::try_from(Value::Bool(revoke_polkit(uid, &action))).unwrap()])
    }
}

fn block_authorization(args: &[OwnedValue]) -> Result<Vec<OwnedValue>> {
    let uid = arg_to_u32(args, 0).unwrap_or(u32::MAX);
    let action = arg_to_string_opt(args, 1).unwrap_or_default();
    Ok(vec![OwnedValue::try_from(Value::Bool(block_polkit(uid, &action))).unwrap()])
}

fn list_user_authorizations(args: &[OwnedValue]) -> Result<Vec<OwnedValue>> {
    let _uid = arg_to_u32(args, 0).unwrap_or(u32::MAX);
    // PolicyKit authorization listing requires D-Bus polkitd communication
    // TODO: implement via zbus polkit interface
    Ok(vec![OwnedValue::try_from(Value::Array(zvariant::Array::new(
        zvariant::Signature::from_static_str_unchecked("v"),
    )))
    .unwrap()])
}

// ── PolicyKit yardımcıları (stub) ───────────────────────────────────────

fn grant_polkit(_uid: u32, _action: &str) -> bool {
    // TODO: Implement via polkit D-Bus API or pkexec
    true
}

fn revoke_polkit(_uid: u32, _action: &str) -> bool {
    true
}

fn revoke_polkit_all(_uid: u32) -> bool {
    true
}

fn block_polkit(_uid: u32, _action: &str) -> bool {
    true
}
