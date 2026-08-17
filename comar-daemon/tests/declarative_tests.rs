use comar_daemon::{
    config::{Config, DBUS_SERVER_ADDRESS, DBUS_SERVICE_NAME, DEFAULT_IDLE_TIMEOUT},
    script::execute,
};
use std::fs;
use zvariant::Value;
use std::ops::Deref;

fn test_cfg(tmp: &std::path::Path) -> Config {
    Config {
        service_name:   DBUS_SERVICE_NAME.into(),
        dir_data:       tmp.to_path_buf(),
        dir_log:        tmp.join("log"),
        debug:          false,
        ignore_missing: false,
        print_mode:     false,
        server_address: DBUS_SERVER_ADDRESS.into(),
        timeout:        DEFAULT_IDLE_TIMEOUT,
    }
}

#[test]
fn execute_kdl_service_info() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = test_cfg(tmp.path());
    let scripts_dir = cfg.dir_scripts().join("System.Service");
    fs::create_dir_all(&scripts_dir).unwrap();

    let kdl_content = r#"
service "test_svc" {
    type "script"
    description "Test KDL Service"
    status "true"
}
"#;
    fs::write(scripts_dir.join("test_svc.kdl"), kdl_content).unwrap();

    let result = execute(&cfg, Some("test_svc"), "System.Service", "info", &[]).unwrap();
    assert_eq!(result.len(), 3);
    if let Value::Str(s) = result[0].deref() { assert_eq!(s.as_str(), "script"); }
    if let Value::Str(s) = result[1].deref() { assert_eq!(s.as_str(), "Test KDL Service"); }
    if let Value::Str(s) = result[2].deref() { assert_eq!(s.as_str(), "on"); } // "true" exit code 0 means "on"
}

#[test]
fn execute_json_service_info() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = test_cfg(tmp.path());
    let scripts_dir = cfg.dir_scripts().join("System.Service");
    fs::create_dir_all(&scripts_dir).unwrap();

    let json_content = r#"{
        "service": {
            "type": "systemd",
            "description": "Test JSON Service"
        },
        "commands": {
            "status": "false"
        }
    }"#;
    fs::write(scripts_dir.join("test_svc.json"), json_content).unwrap();

    let result = execute(&cfg, Some("test_svc"), "System.Service", "info", &[]).unwrap();
    assert_eq!(result.len(), 3);
    if let Value::Str(s) = result[0].deref() { assert_eq!(s.as_str(), "systemd"); }
    if let Value::Str(s) = result[1].deref() { assert_eq!(s.as_str(), "Test JSON Service"); }
    if let Value::Str(s) = result[2].deref() { assert_eq!(s.as_str(), "off"); } // "false" exit code 1 means "off"
}

#[test]
fn execute_declarative_command_success() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = test_cfg(tmp.path());
    let scripts_dir = cfg.dir_scripts().join("System.Service");
    fs::create_dir_all(&scripts_dir).unwrap();

    let kdl_content = r#"
service "cmd_test" {
    description "Command Test"
    start "true"
}
"#;
    fs::write(scripts_dir.join("cmd_test.kdl"), kdl_content).unwrap();

    let result = execute(&cfg, Some("cmd_test"), "System.Service", "start", &[]);
    assert!(result.is_ok());
}

#[test]
fn execute_declarative_command_fail() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = test_cfg(tmp.path());
    let scripts_dir = cfg.dir_scripts().join("System.Service");
    fs::create_dir_all(&scripts_dir).unwrap();

    let kdl_content = r#"
service "fail_test" {
    description "Fail Test"
    start "false"
}
"#;
    fs::write(scripts_dir.join("fail_test.kdl"), kdl_content).unwrap();

    let result = execute(&cfg, Some("fail_test"), "System.Service", "start", &[]);
    assert!(result.is_err());
}
