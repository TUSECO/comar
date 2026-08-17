//! Python script motoru testleri.
//!
//! `script.rs`'deki betik çalıştırma, tip dönüşümü ve
//! CSL built-in fonksiyonlarını doğrular.

use comar_daemon::{
    config::{Config, DBUS_SERVER_ADDRESS, DBUS_SERVICE_NAME, DEFAULT_IDLE_TIMEOUT},
    script::{
        clear_request_context, dbus_to_python,
        execute, finalize, init, python_to_dbus, set_request_context,
        RequestContext,
    },
};
use std::ops::Deref;
use pyo3::prelude::*;
use std::fs;
use zvariant::{OwnedValue, Value};

// ── Test altyapısı ────────────────────────────────────────────────────────────

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

fn setup_ctx(path: &str, model: &str, method: &str) {
    set_request_context(RequestContext {
        path:         path.into(),
        interface:    format!("{}.{}", DBUS_SERVICE_NAME, model),
        member:       method.into(),
        sender:       ":1.42".into(),
        lang:         "en".into(),
        data_dir:     "/var/db/comar3".into(),
        iface_prefix: DBUS_SERVICE_NAME.into(),
    });
}

fn make_script(_tmp: &std::path::Path, cfg: &Config, app: &str, model: &str, code: &str) {
    let dir = cfg.dir_scripts().join(model);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{}.py", app)), code).unwrap();
}

// ── Tip dönüşümü: D-Bus → Python ─────────────────────────────────────────────

#[test]
fn dbus_string_roundtrip() {
    Python::with_gil(|py| {
        let val = OwnedValue::try_from(Value::Str("merhaba".into())).unwrap();
        let py_vals = dbus_to_python(py, &[val]).unwrap();
        assert_eq!(py_vals[0].extract::<String>().unwrap(), "merhaba");
    });
}

#[test]
fn dbus_int32_roundtrip() {
    Python::with_gil(|py| {
        let val = OwnedValue::try_from(Value::I32(42)).unwrap();
        let py_vals = dbus_to_python(py, &[val]).unwrap();
        assert_eq!(py_vals[0].extract::<i32>().unwrap(), 42);
    });
}

#[test]
fn dbus_bool_true_roundtrip() {
    Python::with_gil(|py| {
        let val = OwnedValue::try_from(Value::Bool(true)).unwrap();
        let py_vals = dbus_to_python(py, &[val]).unwrap();
        assert!(py_vals[0].extract::<bool>().unwrap());
    });
}

#[test]
fn dbus_bool_false_roundtrip() {
    Python::with_gil(|py| {
        let val = OwnedValue::try_from(Value::Bool(false)).unwrap();
        let py_vals = dbus_to_python(py, &[val]).unwrap();
        assert!(!py_vals[0].extract::<bool>().unwrap());
    });
}

#[test]
fn dbus_float_roundtrip() {
    Python::with_gil(|py| {
        let val = OwnedValue::try_from(Value::F64(3.14)).unwrap();
        let py_vals = dbus_to_python(py, &[val]).unwrap();
        let f: f64 = py_vals[0].extract().unwrap();
        assert!((f - 3.14).abs() < 1e-10);
    });
}

#[test]
fn dbus_multiple_args() {
    Python::with_gil(|py| {
        let args = vec![
            OwnedValue::try_from(Value::Str("root".into())).unwrap(),
            OwnedValue::try_from(Value::I32(0)).unwrap(),
            OwnedValue::try_from(Value::Bool(true)).unwrap(),
        ];
        let py_vals = dbus_to_python(py, &args).unwrap();
        assert_eq!(py_vals.len(), 3);
        assert_eq!(py_vals[0].extract::<String>().unwrap(), "root");
        assert_eq!(py_vals[1].extract::<i32>().unwrap(), 0);
        assert!(py_vals[2].extract::<bool>().unwrap());
    });
}

// ── Tip dönüşümü: Python → D-Bus ─────────────────────────────────────────────

#[test]
fn python_string_to_dbus() {
    Python::with_gil(|py| {
        let obj = "dünya".to_object(py).into_bound(py).into_any();
        let vals = python_to_dbus(py, &obj).unwrap();
        assert_eq!(vals.len(), 1);
        assert!(matches!(vals[0].deref(), Value::Str(_)));
    });
}

#[test]
fn python_int_small_to_i32() {
    Python::with_gil(|py| {
        let obj = 42i32.to_object(py).into_bound(py).into_any();
        let vals = python_to_dbus(py, &obj).unwrap();
        assert!(matches!(vals[0].deref(), Value::I32(42)));
    });
}

#[test]
fn python_int_large_to_i64() {
    Python::with_gil(|py| {
        let big: i64 = 3_000_000_000;
        let obj = big.to_object(py).into_bound(py).into_any();
        let vals = python_to_dbus(py, &obj).unwrap();
        assert!(matches!(vals[0].deref(), Value::I64(_)));
    });
}

#[test]
fn python_bool_to_dbus() {
    Python::with_gil(|py| {
        let obj = true.to_object(py).into_bound(py).into_any();
        let vals = python_to_dbus(py, &obj).unwrap();
        assert!(matches!(vals[0].deref(), Value::Bool(true)));
    });
}

#[test]
fn python_none_to_empty_string() {
    Python::with_gil(|py| {
        let obj = py.None().into_bound(py);
        let vals = python_to_dbus(py, &obj).unwrap();
        if let Value::Str(s) = vals[0].deref() {
            assert_eq!(s.as_str(), "");
        } else {
            panic!("None → boş string bekleniyor");
        }
    });
}

// ── CSL built-in'leri ─────────────────────────────────────────────────────────

#[test]
fn csl_bus_path_returns_context_path() {
    use comar_daemon::script::inject_csl_builtins_for_test;

    setup_ctx("/package/apache", "System.Service", "start");
    Python::with_gil(|py| {
        inject_csl_builtins_for_test(py).unwrap();
        let builtins = py.import_bound("builtins").unwrap();
        let bus_path = builtins.getattr("bus_path").unwrap();
        let result: String = bus_path.call0().unwrap().extract().unwrap();
        assert_eq!(result, "/package/apache");
    });
    clear_request_context();
}

#[test]
fn csl_model_extracts_from_interface() {
    use comar_daemon::script::inject_csl_builtins_for_test;

    setup_ctx("/package/apache", "System.Service", "start");
    Python::with_gil(|py| {
        inject_csl_builtins_for_test(py).unwrap();
        let builtins = py.import_bound("builtins").unwrap();
        let model_fn = builtins.getattr("model").unwrap();
        let result: String = model_fn.call0().unwrap().extract().unwrap();
        assert_eq!(result, "System.Service");
    });
    clear_request_context();
}

#[test]
fn csl_package_extracts_from_path() {
    use comar_daemon::script::inject_csl_builtins_for_test;

    setup_ctx("/package/nginx", "System.Service", "stop");
    Python::with_gil(|py| {
        inject_csl_builtins_for_test(py).unwrap();
        let builtins = py.import_bound("builtins").unwrap();
        let pkg_fn = builtins.getattr("package").unwrap();
        let result: String = pkg_fn.call0().unwrap().extract().unwrap();
        assert_eq!(result, "nginx");
    });
    clear_request_context();
}

#[test]
fn csl_fail_raises_exception() {
    use comar_daemon::script::inject_csl_builtins_for_test;

    setup_ctx("/", "Core", "register");
    Python::with_gil(|py| {
        inject_csl_builtins_for_test(py).unwrap();
        let builtins = py.import_bound("builtins").unwrap();
        let fail_fn  = builtins.getattr("fail").unwrap();
        let result   = fail_fn.call1(("servis başlatılamadı",));
        assert!(result.is_err(), "fail() exception fırlatmalı");
    });
    clear_request_context();
}

// ── Script çalıştırma ─────────────────────────────────────────────────────────

#[test]
fn execute_simple_method_returns_string() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = test_cfg(tmp.path());

    make_script(&tmp.path(), &cfg, "apache", "System.Service", r#"
def info():
    return "server", "Apache", "stopped"
"#);

    setup_ctx("/package/apache", "System.Service", "info");
    let result = execute(&cfg, Some("apache"), "System.Service", "info", &[]);
    clear_request_context();

    let vals = result.unwrap();
    assert_eq!(vals.len(), 3);
    if let Value::Str(s) = vals[0].deref() { assert_eq!(s.as_str(), "server"); }
    if let Value::Str(s) = vals[1].deref() { assert_eq!(s.as_str(), "Apache"); }
    if let Value::Str(s) = vals[2].deref() { assert_eq!(s.as_str(), "stopped"); }
}

#[test]
fn execute_method_with_args() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = test_cfg(tmp.path());

    make_script(&tmp.path(), &cfg, "pkg", "User.Manager", r#"
def echo(name, uid):
    return name, uid
"#);

    setup_ctx("/package/pkg", "User.Manager", "echo");
    let args = vec![
        OwnedValue::try_from(Value::Str("ali".into())).unwrap(),
        OwnedValue::try_from(Value::I32(1001)).unwrap(),
    ];
    let result = execute(&cfg, Some("pkg"), "User.Manager", "echo", &args);
    clear_request_context();

    let vals = result.unwrap();
    assert_eq!(vals.len(), 2);
    if let Value::Str(s) = vals[0].deref() { assert_eq!(s.as_str(), "ali"); }
    if let Value::I32(n) = vals[1].deref() { assert_eq!(*n, 1001); }
}

#[test]
fn execute_missing_method_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = test_cfg(tmp.path());

    make_script(&tmp.path(), &cfg, "pkg", "System.Service", "# boş betik");

    setup_ctx("/package/pkg", "System.Service", "start");
    let result = execute(&cfg, Some("pkg"), "System.Service", "start", &[]);
    clear_request_context();

    assert!(result.is_err(), "Tanımsız method hata döndürmeli");
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("start") || err_str.contains("Missing"),
        "Hata mesajında method adı bulunmalı: {}", err_str
    );
}

#[test]
fn execute_ignore_missing_returns_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = test_cfg(tmp.path());
    cfg.ignore_missing = true;

    make_script(&tmp.path(), &cfg, "pkg", "System.Service", "# boş betik");

    setup_ctx("/package/pkg", "System.Service", "reload");
    let result = execute(&cfg, Some("pkg"), "System.Service", "reload", &[]);
    clear_request_context();

    assert!(result.is_ok(), "ignore_missing=true hata döndürmemeli");
    assert!(result.unwrap().is_empty());
}

#[test]
fn execute_script_not_found_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = test_cfg(tmp.path());

    setup_ctx("/package/ghost", "System.Service", "start");
    let result = execute(&cfg, Some("ghost"), "System.Service", "start", &[]);
    clear_request_context();

    assert!(result.is_err());
}

#[test]
fn execute_python_exception_propagates() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = test_cfg(tmp.path());

    make_script(&tmp.path(), &cfg, "pkg", "System.Service", r#"
def start():
    raise RuntimeError("servis başlatılamadı")
"#);

    setup_ctx("/package/pkg", "System.Service", "start");
    let result = execute(&cfg, Some("pkg"), "System.Service", "start", &[]);
    clear_request_context();

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("servis başlatılamadı"));
}

// ── Python 3 uyumluluk ────────────────────────────────────────────────────────

#[test]
fn compat_replaces_basestring() {
    use comar_daemon::script::compat_python3;
    let code = "isinstance(x, basestring)";
    assert!(compat_python3(code).contains("str"));
}

#[test]
fn compat_replaces_iteritems() {
    use comar_daemon::script::compat_python3;
    let code = "for k, v in d.iteritems(): pass";
    assert!(compat_python3(code).contains(".items()"));
}

#[test]
fn compat_replaces_file_builtin() {
    use comar_daemon::script::compat_python3;
    let code = "f = file('/tmp/x', 'w')";
    assert!(compat_python3(code).contains("open("));
}

#[test]
fn compat_print_statement_wrapped_with_parens() {
    use comar_daemon::script::compat_python3;
    // Python 2: print "merhaba"  →  Python 3: print("merhaba")
    let code = "print \"merhaba\"";
    let out  = compat_python3(code);
    assert!(out.starts_with("print("), "print() ile başlamalı: {}", out);
    assert!(out.ends_with(')'),        "kapanış parantezi olmalı: {}", out);
}

#[test]
fn compat_print_already_call_untouched() {
    use comar_daemon::script::compat_python3;
    let code = "print(\"zaten çağrı\")";
    let out  = compat_python3(code);
    // Çift parantez olmamalı
    assert!(!out.contains("print(("), "çift parantez olmamalı: {}", out);
}
