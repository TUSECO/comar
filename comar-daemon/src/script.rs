//! Python script motoru — script.c + pydbus.c karşılığı.
//!
//! # Sorumluluklar
//!
//! * Python VM başlatma/kapatma (`script_init` / `script_finalize`)
//! * CSL built-in'lerini `__builtins__`'e enjekte etme (14 fonksiyon)
//! * Betik yükleme ve metod çağrısı (`py_execute`)
//! * Python ↔ zvariant::Value tip dönüşümü (`pydbus_import` / `pydbus_export`)
//! * Python exception yakalama (`py_catch`)
//!
//! # Thread güvenliği
//!
//! Her D-Bus isteği `tokio::task::spawn_blocking` içinde çalışır.
//! Python GIL `Python::with_gil()` ile her çağrıda alınır.
//! İstek bağlamı (path, interface, method, sender) thread-local
//! depolama ile CSL fonksiyonlarına aktarılır.

use crate::{
    config::Config,
    db::ModelRegistry,
    error::{ComarError, Result},
};
use rust_i18n::t;
use pyo3::{
    prelude::*,
    types::{PyAnyMethods, PyBool, PyDict, PyList, PyModule, PyTuple},
    types::PyCFunction,
};
use std::{
    cell::RefCell,
    path::Path,
    sync::Arc
};
use tracing::debug;
use zvariant::{OwnedValue, Value};

// ── İstek bağlamı (thread-local) ─────────────────────────────────────────────
//
// C'de `my_proc.bus_msg` global process değişkeni kullanılıyordu.
// Rust'ta fork() yerine spawn_blocking kullandığımız için
// thread-local depolama ile aynı etkiyi sağlıyoruz.

thread_local! {
    /// Geçerli D-Bus isteğinin bağlamı.
    /// CSL fonksiyonları tarafından okunur.
    static REQUEST_CTX: RefCell<Option<RequestContext>> = RefCell::new(None);
}

/// Tek bir D-Bus isteğinin bağlam bilgileri.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// D-Bus nesne yolu  (örn. `/package/apache`)
    pub path:      String,
    /// D-Bus arayüzü   (örn. `tr.org.luppo.comar.System.Service`)
    pub interface: String,
    /// D-Bus üye adı   (örn. `start`)
    pub member:    String,
    /// D-Bus gönderici (örn. `:1.42`)
    pub sender:    String,
    /// Çözümlenmiş dil kodu (örn. `"tr"`, `"en"`)
    /// `setLocale` tarafından belirlenir; varsayılan `"en"`.
    pub lang:      String,
    /// Veri kök dizini (örn. `/var/db/comar3`)
    pub data_dir:  String,
    /// D-Bus arayüz öneki
    pub iface_prefix: String,
}

/// Geçerli isteğin bağlamını belirler.
/// `py_execute` çağrısından önce çağrılmalıdır.
pub fn set_request_context(ctx: RequestContext) {
    REQUEST_CTX.with(|c| *c.borrow_mut() = Some(ctx));
}

/// Geçerli isteğin bağlamını temizler.
pub fn clear_request_context() {
    REQUEST_CTX.with(|c| *c.borrow_mut() = None);
}

fn with_ctx<F, T>(f: F) -> Option<T>
where
    F: FnOnce(&RequestContext) -> T,
{
    REQUEST_CTX.with(|c| c.borrow().as_ref().map(f))
}

// ── Python VM ─────────────────────────────────────────────────────────────────

/// Python VM'i başlatır.
///
/// `script_init()` karşılığı.
pub fn init(_cfg: &Config, _registry: Arc<ModelRegistry>) -> Result<()> {
    // PyO3 "auto-initialize" feature etkin olduğundan
    // Python VM ilk `Python::with_gil()` çağrısında otomatik başlar.
    // Burada sadece sağlık kontrolü yapıyoruz.
    Python::with_gil(|py| {
        let version = py.version();
        debug!("Python VM hazır: {}", version);
        Ok::<(), ComarError>(())
    })?;
    Ok(())
}

/// Python VM'i kapatır.
///
/// `script_finalize()` karşılığı.
pub fn finalize() {
    debug!("Python VM kapatılıyor...");
    // PyO3 auto-initialize modunda GIL'i bırakmak yeterli;
    // Python interpreter process çıkışında temizlenir.
}

// ── Ana çalıştırıcı ───────────────────────────────────────────────────────────

/// Python betiğini yükleyip belirtilen methodu çalıştırır.
///
/// `py_execute()` karşılığı.
///
/// * `app = None`  → `modules/core.py`'den metod çalıştırılır
/// * `app = Some(x)` → `scripts/<model>/<app>.py`'den metod çalıştırılır
///
/// Dönüş: Python'dan gelen değer, zvariant::OwnedValue listesi olarak.
pub fn execute(
    cfg:    &Config,
    app:    Option<&str>,
    model:  &str,
    method: &str,
    args:   &[OwnedValue],
) -> Result<Vec<OwnedValue>> {
    let cfg_scripts_dir   = cfg.dir_scripts().to_string_lossy().to_string();
    let cfg_modules_dir   = cfg.dir_modules().to_string_lossy().to_string();
    let ignore_missing    = cfg.ignore_missing;

    let app_owned     = app.map(|s| s.to_string());
    let model_owned   = model.to_string();
    let method_owned  = method.to_string();

    Python::with_gil(|py| {
        let lang_owned = REQUEST_CTX.with(|ctx| {
            ctx.borrow()
                .as_ref()
                .map(|c| c.lang.clone())
                .unwrap_or_else(|| "tr".into())
        });

        // ── sys.path'e modüller dizinini ekle ──────────────────────────────
        // C: PyList_Insert(sys.path, 0, config_dir_modules)
        let sys  = py.import_bound("sys").map_err(pyo3_err)?;
        let path = sys.getattr("path").map_err(pyo3_err)?;
        let path_list: Vec<String> = path.extract().map_err(pyo3_err)?;
        if !path_list.contains(&cfg_modules_dir) {
            path.call_method1("insert", (0, cfg_modules_dir.as_str()))
                .map_err(pyo3_err)?;
        }

        // ── CSL built-in'lerini __builtins__'e enjekte et ─────────────────
        inject_csl_builtins(py)?;

        // ── Scripti yükle ve methodu çağır ────────────────────────────────
        let py_func = if let Some(ref app_name) = app_owned {
            let script_base = format!("{}/{}/{}", cfg_scripts_dir, model_owned, app_name);
            
            if Path::new(&format!("{}.kdl", script_base)).exists() {
                let script_path = std::path::PathBuf::from(format!("{}.kdl", script_base));
                return crate::declarative::execute(&script_path, &method_owned, args, &lang_owned);
            } else if Path::new(&format!("{}.py", script_base)).exists() {
                let script_path = format!("{}.py", script_base);
                load_script_function(py, &script_path, &method_owned, ignore_missing)?
            } else if Path::new(&format!("{}.json", script_base)).exists() {
                let script_path = std::path::PathBuf::from(format!("{}.json", script_base));
                return crate::declarative::execute(&script_path, &method_owned, args, &lang_owned);
            } else {
                if ignore_missing {
                    return Ok(vec![]);
                }
                return Err(ComarError::Missing(t!(
                    "error_missing_script",
                    locale = &lang_owned,
                    path = script_base
                ).to_string()));
            }
        } else {
            // modules/core.py'den import et
            let core_mod = py.import_bound("core").map_err(|e| {
                ComarError::Script(format!("core modülü yüklenemedi: {}", e))
            })?;
            let func = core_mod.getattr(method_owned.as_str()).ok();
            func.map(|f| f.into())
        };

        // ── Methodu çağır ─────────────────────────────────────────────────
        match py_func {
            None if ignore_missing => {
                // --ignore-missing: None döndür
                Ok(vec![])
            }
            None => Err(ComarError::Missing(format!(
                "Method '{}' script'te tanımlı değil",
                method_owned
            ))),
            Some(func) => {
                // D-Bus argümanlarını Python'a çevir
                let py_args = dbus_to_python(py, args)?;
                let py_tuple = PyTuple::new_bound(py, py_args.iter());

                // policy_action_id attribute kontrolü (runtime PolicyKit)
                // C: PyObject_HasAttrString(py_func, "policy_action_id")
                if func.hasattr("policy_action_id").unwrap_or(false) {
                    if let Ok(aid) = func.getattr("policy_action_id") {
                        let action_id: String = aid.extract().unwrap_or_default();
                        if !action_id.is_empty() {
                            // PolicyKit kontrolü burada değil process.rs'de yapılır
                            // (async context gerektirir). Bu flag işaretlenir.
                            debug!("Runtime PolicyKit action: {}", action_id);
                        }
                    }
                }

                // Methodu çağır
                let result = func.call1(py_tuple).map_err(|e| {
                    let msg = format_python_error(py, &e);
                    ComarError::Script(msg)
                })?;

                // Sonucu Python'dan D-Bus'a çevir
                let values = python_to_dbus(py, result.as_any())?;
                Ok(values)
            }
        }
    })
}

// ── Script yükleme ────────────────────────────────────────────────────────────

/// Betik dosyasını yükler ve belirtilen fonksiyonu döndürür.
///
/// C'deki: `access(fn_script) + load_file + Py_CompileString + PyImport_ExecCodeModule`
fn load_script_function<'py>(
    py:            Python<'py>,
    script_path:   &str,
    method_name:   &str,
    ignore_missing: bool,
) -> Result<Option<Bound<'py, PyAny>>> {
    if !Path::new(script_path).exists() {
        if ignore_missing {
            return Ok(None);
        }
        return Err(ComarError::Missing(format!(
            "Betik bulunamadı: {}",
            script_path
        )));
    }

    let code = std::fs::read_to_string(script_path).map_err(|e| {
        ComarError::Script(format!("Betik okunamadı {}: {}", script_path, e))
    })?;

    // Python 3 uyumluluğu için küçük düzeltmeler (Python 2 → 3)
    let code = compat_python3(&code);

    // Betigi belirtilen dosya yolu adıyla bir modül olarak derle ve çalıştır
    // Modül adının çakışmaması için dosya yolunu kullanıyoruz.
    let module_code = PyModule::from_code_bound(py, &code, script_path, script_path)
        .map_err(|e| {
            ComarError::Script(format!("Betik derlenemedi {}: {}", script_path, e))
        })?;

    // Method'u ara
    Ok(module_code.getattr(method_name).ok().map(|f| f.into()))
}

/// Temel Python 2 → 3 uyumluluk düzeltmeleri.
/// (comar-api'nin mevcut Python 2 betikleri için)
pub fn compat_python3(code: &str) -> String {
    // `print expr` → `print(expr)` satır satır dönüşümü
    // Yalnızca bağımsız `print` ifadelerini (string/yorum içinde değil) dönüştürür.
    let code = convert_print_statements(code);
    code
        .replace("basestring", "str")
        .replace(".iteritems()", ".items()")
        .replace(".itervalues()", ".values()")
        .replace(".iterkeys()", ".keys()")
        .replace("has_key(", "__contains__(")
        // file() built-in Python 3'te yok
        .replace("file(", "open(")
}

/// `print expr` kalıplarını `print(expr)` biçimine çevirir.
/// Her satırı ayrı ayrı işleyerek yorum ve string karışıklığını azaltır.
fn convert_print_statements(code: &str) -> String {
    code.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            // Yorum satırları ve boş satırlar dokunulmadan geçer
            if trimmed.starts_with('#') || trimmed.is_empty() {
                return line.to_string();
            }
            // Zaten print(...) biçimindeyse dokunma
            if trimmed.starts_with("print(") {
                return line.to_string();
            }
            // `print expr` kalıbı: satır `print ` ile başlıyorsa dönüştür
            if let Some(rest) = trimmed.strip_prefix("print ") {
                let indent_len = line.len() - trimmed.len();
                let indent = &line[..indent_len];
                let rest = rest.trim_end();
                // Çok değer varsa veya virgül içeriyorsa tuple gibi sarılır
                return format!("{}print({})", indent, rest);
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── CSL Built-in'ler ──────────────────────────────────────────────────────────
//
// C'deki PyMethodDef methods[] dizisindeki 14 fonksiyonun karşılığı.
// __builtins__ dict'ine enjekte edilir, böylece betikler direkt çağırabilir.

/// Sadece testler için CSL built-in'lerini enjekte eder.
#[allow(dead_code)]
pub fn inject_csl_builtins_for_test(py: Python<'_>) -> Result<()> { inject_csl_builtins(py) }

fn inject_csl_builtins(py: Python<'_>) -> Result<()> {
    let builtins = py.import_bound("builtins").map_err(pyo3_err)?;

    // model() — D-Bus arayüzünden model adını çıkarır
    let model_fn = PyCFunction::new_closure_bound(py, None, None, move |_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
        Python::with_gil(|py| {
            let val = with_ctx(|ctx| {
                ctx.interface
                    .strip_prefix(&ctx.iface_prefix)
                    .and_then(|s| s.strip_prefix('.'))
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
            Ok::<PyObject, PyErr>(val.to_object(py))
        })
    })
    .map_err(pyo3_err)?;
    builtins.setattr("model", model_fn).map_err(pyo3_err)?;

    // package() / script() — D-Bus yolundan uygulama adını çıkarır
    let package_fn = PyCFunction::new_closure_bound(py, None, None, move |_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
        Python::with_gil(|py| {
            let val = with_ctx(|ctx| {
                ctx.path
                    .strip_prefix("/package/")
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            })
            .flatten();
            Ok::<PyObject, PyErr>(match val {
                Some(s) => s.to_object(py),
                None    => py.None(),
            })
        })
    })
    .map_err(pyo3_err)?;
    builtins.setattr("package",  &package_fn).map_err(pyo3_err)?;
    builtins.setattr("script",   &package_fn).map_err(pyo3_err)?; // alias

    // bus_path()
    let bus_path_fn = PyCFunction::new_closure_bound(py, None, None, |_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
        Python::with_gil(|py| {
            let v = with_ctx(|ctx| ctx.path.clone()).unwrap_or_default();
            Ok::<PyObject, PyErr>(v.to_object(py))
        })
    })
    .map_err(pyo3_err)?;
    builtins.setattr("bus_path", bus_path_fn).map_err(pyo3_err)?;

    // bus_interface()
    let bus_iface_fn = PyCFunction::new_closure_bound(py, None, None, |_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
        Python::with_gil(|py| {
            let v = with_ctx(|ctx| ctx.interface.clone()).unwrap_or_default();
            Ok::<PyObject, PyErr>(v.to_object(py))
        })
    })
    .map_err(pyo3_err)?;
    builtins.setattr("bus_interface", bus_iface_fn).map_err(pyo3_err)?;

    // bus_member()
    let bus_member_fn = PyCFunction::new_closure_bound(py, None, None, |_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
        Python::with_gil(|py| {
            let v = with_ctx(|ctx| ctx.member.clone()).unwrap_or_default();
            Ok::<PyObject, PyErr>(v.to_object(py))
        })
    })
    .map_err(pyo3_err)?;
    builtins.setattr("bus_member", bus_member_fn).map_err(pyo3_err)?;

    // bus_sender()
    let bus_sender_fn = PyCFunction::new_closure_bound(py, None, None, |_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
        Python::with_gil(|py| {
            let v = with_ctx(|ctx| ctx.sender.clone()).unwrap_or_default();
            Ok::<PyObject, PyErr>(v.to_object(py))
        })
    })
    .map_err(pyo3_err)?;
    builtins.setattr("bus_sender", bus_sender_fn).map_err(pyo3_err)?;

    // config_datapath()
    let data_dir = with_ctx(|ctx| ctx.data_dir.clone()).unwrap_or_default();
    let cfg_datapath_fn = PyCFunction::new_closure_bound(py, None, None, move |_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
        let v = data_dir.clone();
        Python::with_gil(|py| Ok::<PyObject, PyErr>(v.to_object(py)))
    })
    .map_err(pyo3_err)?;
    builtins.setattr("config_datapath", cfg_datapath_fn).map_err(pyo3_err)?;

    // config_interface()
    let iface_prefix = with_ctx(|ctx| ctx.iface_prefix.clone()).unwrap_or_default();
    let cfg_iface_fn = PyCFunction::new_closure_bound(py, None, None, move |_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
        let v = iface_prefix.clone();
        Python::with_gil(|py| Ok::<PyObject, PyErr>(v.to_object(py)))
    })
    .map_err(pyo3_err)?;
    builtins.setattr("config_interface", cfg_iface_fn).map_err(pyo3_err)?;

    // _(dict) — i18n: dil koduna göre çeviriye bak
    // C: c_i18n — setLocale ile kaydedilen dil koduna göre doğru çeviriyi döndürür.
    let i18n_fn = PyCFunction::new_closure_bound(py, None, None, |args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
        Python::with_gil(|py| {
            let dict: Bound<PyDict> = args.get_item(0)
                .and_then(|a| a.downcast::<PyDict>().map_err(PyErr::from).map(|d| d.clone()))
                .map_err(|_| pyo3::exceptions::PyValueError::new_err("dict bekleniyor"))?;

            // Çözümlenmiş dil kodunu thread-local bağlamdan al (setLocale'den gelir)
            let lang = with_ctx(|ctx| ctx.lang.clone()).unwrap_or_else(|| "en".into());

            if let Ok(Some(v)) = dict.get_item(lang.as_str()) {
                let obj: PyObject = v.to_object(py);
                return Ok(obj);
            }
            if let Ok(Some(v)) = dict.get_item("en") {
                let obj: PyObject = v.to_object(py);
                return Ok(obj);
            }
            Err(pyo3::exceptions::PyKeyError::new_err("Varsayılan 'en' çevirisi eksik"))
        })
    })
    .map_err(pyo3_err)?;
    builtins.setattr("_", i18n_fn).map_err(pyo3_err)?;

    // notify(model, signal, *args) — D-Bus sinyali yayınlar.
    // C: c_notify() → bus_signal()
    // signal_bus kanalı üzerinden async D-Bus döngüsüne iletilir.
    let notify_fn = PyCFunction::new_closure_bound(py, None, None, |args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
        Python::with_gil(|py| {
            let model:  String = args.get_item(0).and_then(|a| a.extract::<String>())
                .unwrap_or_default();
            let signal: String = args.get_item(1).and_then(|a| a.extract::<String>())
                .unwrap_or_default();
            let path = with_ctx(|ctx| ctx.path.clone()).unwrap_or_else(|| "/".into());

            // Ek argümanları D-Bus değerlerine çevir
            let extra_args: Vec<zvariant::OwnedValue> = (2..args.len())
                .filter_map(|i| args.get_item(i).ok())
                .filter_map(|a| {
                    python_to_dbus(py, &a).ok().and_then(|mut v| v.pop())
                })
                .collect();

            crate::signal_bus::send_signal(crate::signal_bus::SignalRequest {
                path, model, signal, args: extra_args,
            });

            Ok::<PyObject, PyErr>(py.None())
        })
    })
    .map_err(pyo3_err)?;
    builtins.setattr("notify", notify_fn).map_err(pyo3_err)?;

    // call(app, model, method, args=None, timeout=-1) — iç ÇOMAR çağrısı.
    // C: c_call() → bus_call() → bus_execute()
    // signal_bus kanalı üzerinden async D-Bus döngüsüne iletilir;
    // yanıt sync mpsc ile beklenir (spawn_blocking içinde güvenli).
    let call_fn = PyCFunction::new_closure_bound(py, None, None, |args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
        Python::with_gil(|py| {
            let app:     String = args.get_item(0).and_then(|a| a.extract::<String>())
                .unwrap_or_default();
            let model:   String = args.get_item(1).and_then(|a| a.extract::<String>())
                .unwrap_or_default();
            let method:  String = args.get_item(2).and_then(|a| a.extract::<String>())
                .unwrap_or_default();
            let timeout: i32   = args.get_item(4).and_then(|a| a.extract::<i32>())
                .unwrap_or(-1);
            // Çözümlenmiş dil kodunu bağlamdan al (setLocale'den gelir)
            let lang = with_ctx(|ctx| ctx.lang.clone()).unwrap_or_else(|| "en".into());

            // Argümanlar (4. parametre, opsiyonel)
            let call_args: Vec<zvariant::OwnedValue> =
                args.get_item(3).ok()
                    .and_then(|a| {
                        python_to_dbus(py, &a).ok()
                    })
                    .unwrap_or_default();

            crate::signal_bus::send_call_and_wait(crate::signal_bus::CallRequest {
                app, model, method, args: call_args, timeout, lang,
            });

            // Yanıt None (async sonuç Python'a aktarılamaz — C'de de aynı)
            Ok::<PyObject, PyErr>(py.None())
        })
    })
    .map_err(pyo3_err)?;
    builtins.setattr("call", call_fn).map_err(pyo3_err)?;

    // fail(msg) — betiği hatayla sonlandırır
    // C: c_fail → PyErr_SetString(PyExc_Exception, strerr)
    let fail_fn = PyCFunction::new_closure_bound(py, None, None, |args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| {
        let msg: String = args.get_item(0).and_then(|a| a.extract::<String>())
            .unwrap_or_else(|_e| "fail() çağrıldı".into());
        Err::<PyObject, _>(pyo3::exceptions::PyRuntimeError::new_err(msg))
    })
    .map_err(pyo3_err)?;
    builtins.setattr("fail", fail_fn).map_err(pyo3_err)?;

    Ok(())
}

// ── Tip dönüşümü: D-Bus (zvariant) → Python ──────────────────────────────────
//
// `pydbus_import` + `py_get_item` + `py_get_dict` + `py_get_list` karşılığı.

/// D-Bus argüman listesini Python nesnelerine çevirir.
pub fn dbus_to_python<'py>(
    py:   Python<'py>,
    vals: &[OwnedValue],
) -> Result<Vec<Bound<'py, PyAny>>> {
    vals.iter()
        .map(|v| value_to_py(py, &**v))
        .collect()
}

// ── Tip dönüşüm yardımcıları ─────────────────────────────────────────────────

#[inline]
fn b_to_py(py: Python<'_>, b: bool) -> Bound<'_, PyAny> {
    pyo3::types::PyBool::new_bound(py, b).as_any().clone()
}
#[inline]
fn i_to_py(py: Python<'_>, n: i64) -> Bound<'_, PyAny> {
    n.to_object(py).into_bound(py).into_any()
}
#[inline]
fn f_to_py(py: Python<'_>, f: f64) -> Bound<'_, PyAny> {
    f.to_object(py).into_bound(py).into_any()
}
#[inline]
fn s_to_py<'py>(py: Python<'py>, s: &str) -> Bound<'py, PyAny> {
    s.to_object(py).into_bound(py).into_any()
}

/// Tek bir zvariant::Value'yu Python nesnesine çevirir.
fn value_to_py<'py>(py: Python<'py>, val: &Value<'_>) -> Result<Bound<'py, PyAny>> {
    let obj: Bound<PyAny> = match val {
        Value::Bool(b)   => b_to_py(py, *b),
        Value::I16(n)    => i_to_py(py, *n as i64),
        Value::U16(n)    => i_to_py(py, *n as i64),
        Value::I32(n)    => i_to_py(py, *n as i64),
        Value::U32(n)    => i_to_py(py, *n as i64),
        Value::I64(n)    => i_to_py(py, *n),
        Value::U64(n)    => i_to_py(py, *n as i64),
        Value::F64(f)    => f_to_py(py, *f),
        Value::Str(s)    => s_to_py(py, s.as_str()),
        Value::ObjectPath(p) => s_to_py(py, p.as_str()),
        Value::Signature(s)  => s_to_py(py, s.as_str()),

        Value::Array(arr) => {
            // Dict mi, list mi?
            if arr.element_signature().starts_with('{') {
                // a{kv} → Python dict
                let dict = PyDict::new_bound(py);
                for item in arr.iter() {
                    if let Value::Structure(kv) = item {
                        let fields: Vec<&Value> = kv.fields().iter().collect();
                        if fields.len() == 2 {
                            let k = value_to_py(py, fields[0])?;
                            let v = value_to_py(py, fields[1])?;
                            dict.set_item(k, v).map_err(pyo3_err)?;
                        }
                    }
                }
                dict.into_any()
            } else {
                // as / ai / … → Python list
                let list = PyList::new_bound(
                    py,
                    arr.iter()
                        .map(|v| value_to_py(py, v))
                        .collect::<Result<Vec<_>>>()?,
                );
                list.into_any()
            }
        }

        Value::Dict(d) => {
            let dict = PyDict::new_bound(py);
            for (k, v) in d.iter() {
                let pk = value_to_py(py, &k)?;
                let pv = value_to_py(py, &v)?;
                dict.set_item(pk, pv).map_err(pyo3_err)?;
            }
            dict.into_any()
        }

        Value::Structure(s) => {
            // (a b c) → Python tuple
            let items: Vec<Bound<PyAny>> = s
                .fields()
                .iter()
                .map(|f| value_to_py(py, f))
                .collect::<Result<_>>()?;
            PyTuple::new_bound(py, items).into_any()
        }

        Value::Value(inner) => {
            // Variant — içini unwrap et
            value_to_py(py, inner)?
        }

        _ => {
            // Bilinmeyen tür → Exception
            return Err(ComarError::Python(format!("Desteklenmeyen tip dönüşümü (D-Bus -> Python): {:?}", val.value_signature())));
        }
    };
    Ok(obj)
}

// ── Tip dönüşümü: Python → D-Bus (zvariant) ──────────────────────────────────
//
// `pydbus_export` + `pydbus_export_item` + `get_obj_sign` karşılığı.

/// Python nesnelerini zvariant::OwnedValue listesine çevirir.
pub fn python_to_dbus(py: Python<'_>, obj: &Bound<PyAny>) -> Result<Vec<OwnedValue>> {
    // Eğer tuple ise her elemanı ayrı ayrı çevir
    if let Ok(tup) = obj.downcast::<PyTuple>() {
        return tup
            .iter()
            .map(|item| py_to_value(py, &item).and_then(|v| v.try_to_owned().map_err(|e| {
                ComarError::Internal(format!("OwnedValue dönüşümü: {}", e))
            })))
            .collect();
    }
    // Tekil değer
    let v = py_to_value(py, obj)?;
    let owned = v.try_to_owned().map_err(|e| {
        ComarError::Internal(format!("OwnedValue dönüşümü: {}", e))
    })?;
    Ok(vec![owned])
}

/// Tek bir Python nesnesini zvariant::Value'ya çevirir.
fn py_to_value<'v>(py: Python<'_>, obj: &Bound<PyAny>) -> Result<Value<'v>> {
    // Sıralama önemli: bool PyInt'ten önce kontrol edilmeli
    if obj.is_none() {
        return Ok(Value::Str("".into())); // C'deki N tipi → boş string
    }
    if let Ok(b) = obj.downcast::<PyBool>() {
        return Ok(Value::Bool(b.is_true()));
    }
    if let Ok(i) = obj.extract::<i64>() {
        // i32 aralığındaysa INT32, değilse INT64
        if i32::try_from(i).is_ok() {
            return Ok(Value::I32(i as i32));
        }
        return Ok(Value::I64(i));
    }
    if let Ok(f) = obj.extract::<f64>() {
        return Ok(Value::F64(f));
    }
    if let Ok(s) = obj.extract::<String>() {
        return Ok(Value::Str(s.into()));
    }
    if let Ok(dict) = obj.downcast::<PyDict>() {
        // Python dict → a{sv} (string key, variant value)
        let mut map: Vec<(Value, Value)> = Vec::new();
        for (k, v) in dict.iter() {
            let dk = py_to_value(py, &k)?;
            let dv = py_to_value(py, &v)?;
            map.push((dk, dv));
        }
        // zvariant Dict oluştur
        let mut d = zvariant::Dict::new(
            zvariant::Signature::try_from("s").unwrap(),
            zvariant::Signature::try_from("v").unwrap(),
        );
        for (k, v) in map {
            d.append(k, Value::Value(Box::new(v)))
                .map_err(|e| ComarError::Internal(format!("Dict oluşturma: {}", e)))?;
        }
        return Ok(Value::Dict(d));
    }
    if let Ok(list) = obj.downcast::<PyList>() {
        let items: Vec<Value> = list
            .iter()
            .map(|i| py_to_value(py, &i))
            .collect::<Result<_>>()?;
        // Homojen array varsayımı: ilk elemanın imzasını kullan
        let sig = items.first()
            .map(|v| v.value_signature().to_owned())
            .unwrap_or_else(|| zvariant::Signature::from_static_str_unchecked("v"));
        let mut arr = zvariant::Array::new(sig);
        for item in items {
            arr.append(item)
                .map_err(|e| ComarError::Internal(format!("Array oluşturma: {}", e)))?;
        }
        return Ok(Value::Array(arr));
    }
    if let Ok(tup) = obj.downcast::<PyTuple>() {
        let fields: Vec<Value> = tup
            .iter()
            .map(|i| py_to_value(py, &i))
            .collect::<Result<_>>()?;
        let mut sb = zvariant::StructureBuilder::new();
        for f in fields { sb.push_value(f); }
        return Ok(Value::Structure(sb.build()));
    }
    // Bilinmeyen → Exception
    Err(ComarError::Python(format!("Desteklenmeyen tip dönüşümü (Python -> D-Bus): {:?}", obj.get_type())))
}

// ── Python exception yakalama ─────────────────────────────────────────────────
//
// `py_catch()` karşılığı.

/// Python istisnasını yakalar ve (hata_adı, mesaj) olarak döndürür.
pub fn catch_exception(py: Python<'_>, err: &PyErr) -> (String, String) {
    let type_name = err
        .get_type_bound(py)
        .name()
        .map(|n| n.to_string())
        .unwrap_or_else(|_| "Exception".to_string());

    let message = err
        .value_bound(py)
        .str()
        .map(|s| s.to_string())
        .unwrap_or_else(|_| String::new());

    (type_name, message)
}

/// PyErr'i insan okunur hata mesajına dönüştürür (traceback dahil).
fn format_python_error(py: Python<'_>, err: &PyErr) -> String {
    let (name, msg) = catch_exception(py, err);
    let mut full_msg = if msg.is_empty() {
        name
    } else {
        format!("{}: {}", name, msg)
    };

    // Traceback ekle
    if let Some(tb) = err.traceback_bound(py) {
        if let Ok(traceback_mod) = py.import_bound("traceback") {
            if let Ok(formatted) = traceback_mod.call_method1("format_exception", (err.get_type_bound(py), err.value_bound(py), tb)) {
                if let Ok(lines) = formatted.downcast::<PyList>() {
                    let tb_text = lines.iter().map(|s| s.to_string()).collect::<Vec<_>>().join("");
                    full_msg = tb_text;
                }
            }
        }
    }
    full_msg
}

// ── Yardımcılar ───────────────────────────────────────────────────────────────

fn pyo3_err(e: PyErr) -> ComarError {
    ComarError::Python(e.to_string())
}

// ── Testler ───────────────────────────────────────────────────────────────────

#[cfg(test)]
use std::ops::Deref; // Deref trait'i sadece testlerde kullanılıyor
mod tests {
    use super::*;

    /// Thread-local bağlam kurulumu
    fn setup_ctx() {
        set_request_context(RequestContext {
            path:         "/package/apache".into(),
            interface:    "tr.org.luppo.comar.System.Service".into(),
            member:       "start".into(),
            sender:       ":1.42".into(),
            lang:         "en".into(),
            data_dir:     "/var/db/comar3".into(),
            iface_prefix: "tr.org.luppo.comar".into(),
        });
    }


    #[test]
    fn ctx_package_name() {
        setup_ctx();
        let pkg = with_ctx(|ctx| {
            ctx.path
                .strip_prefix("/package/")
                .map(|s| s.to_string())
        })
        .flatten();
        assert_eq!(pkg, Some("apache".to_string()));
    }

    #[test]
    fn ctx_model_name() {
        setup_ctx();
        let model = with_ctx(|ctx| {
            ctx.interface
                .strip_prefix(&ctx.iface_prefix)
                .and_then(|s| s.strip_prefix('.'))
                .map(|s| s.to_string())
        })
        .flatten();
        assert_eq!(model, Some("System.Service".to_string()));
    }

    #[test]
    fn dbus_string_to_python() {
        Python::with_gil(|py| {
            let val = Value::Str("merhaba".into());
            let owned = OwnedValue::try_from(val).unwrap();
            let py_vals = dbus_to_python(py, &[owned]).unwrap();
            assert_eq!(py_vals[0].extract::<String>().unwrap(), "merhaba");
        });
    }

    #[test]
    fn dbus_int_to_python() {
        Python::with_gil(|py| {
            let val = Value::I32(42);
            let owned = OwnedValue::try_from(val).unwrap();
            let py_vals = dbus_to_python(py, &[owned]).unwrap();
            assert_eq!(py_vals[0].extract::<i32>().unwrap(), 42);
        });
    }

    #[test]
    fn dbus_bool_to_python() {
        Python::with_gil(|py| {
            let val = Value::Bool(true);
            let owned = OwnedValue::try_from(val).unwrap();
            let py_vals = dbus_to_python(py, &[owned]).unwrap();
            assert!(py_vals[0].extract::<bool>().unwrap());
        });
    }

    #[test]
    fn python_string_to_dbus() {
        Python::with_gil(|py| {
            let obj = "dünya".to_object(py).into_bound(py);
            let vals = python_to_dbus(py, &obj).unwrap();
            assert_eq!(vals.len(), 1);
            if let Value::Str(s) = vals[0].deref() {
                assert_eq!(s.as_str(), "dünya");
            } else {
                panic!("Str bekleniyor");
            }
        });
    }

    #[test]
    fn python_int_to_dbus() {
        Python::with_gil(|py| {
            let obj = 99i32.to_object(py).into_bound(py);
            let vals = python_to_dbus(py, &obj).unwrap();
            assert!(matches!(vals[0].deref(), Value::I32(99)));
        });
    }

    #[test]
    fn python_bool_to_dbus() {
        Python::with_gil(|py| {
            let obj = true.to_object(py).into_bound(py);
            let vals = python_to_dbus(py, &obj).unwrap();
            assert!(matches!(vals[0].deref(), Value::Bool(true)));
        });
    }

    #[test]
    fn inject_csl_builtins_smoke() {
        setup_ctx();
        Python::with_gil(|py| {
            inject_csl_builtins(py).unwrap();
            // bus_path() çalışıyor mu?
            let builtins = py.import_bound("builtins").unwrap();
            let bus_path = builtins.getattr("bus_path").unwrap();
            let result: String = bus_path.call0().unwrap().extract().unwrap();
            assert_eq!(result, "/package/apache");
        });
    }

    #[test]
    fn inject_csl_builtins_model() {
        setup_ctx();
        Python::with_gil(|py| {
            inject_csl_builtins(py).unwrap();
            let builtins = py.import_bound("builtins").unwrap();
            let model_fn = builtins.getattr("model").unwrap();
            let result: String = model_fn.call0().unwrap().extract().unwrap();
            assert_eq!(result, "System.Service");
        });
    }

    #[test]
    fn inject_csl_builtins_package() {
        setup_ctx();
        Python::with_gil(|py| {
            inject_csl_builtins(py).unwrap();
            let builtins = py.import_bound("builtins").unwrap();
            let pkg_fn = builtins.getattr("package").unwrap();
            let result: String = pkg_fn.call0().unwrap().extract().unwrap();
            assert_eq!(result, "apache");
        });
    }

    #[test]
    fn execute_inline_python() {
        // Doğrudan Python kodu çalıştırarak tip dönüşümünü test et
        Python::with_gil(|py| {
            let code = r#"
def test_fn(x, y):
    return x + y
"#;
            let module = PyModule::from_code_bound(py, code, "test.py", "test").unwrap();
            let fn_ = module.getattr("test_fn").unwrap();
            let result = fn_.call1((3i32, 4i32)).unwrap();
            let vals = python_to_dbus(py, &result).unwrap();
            assert!(matches!(vals[0].deref(), Value::I32(7)));
        });
    }

    #[test]
    fn compat_python3_replaces() {
        let code = "basestring\n.iteritems()\nfile(\"/tmp/x\")";
        let out = compat_python3(code);
        assert!(out.contains("str"));
        assert!(out.contains(".items()"));
        assert!(out.contains("open("));
    }
}
