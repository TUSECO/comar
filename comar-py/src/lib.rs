//! ÇOMAR Python binding — PyO3 ile comar-api Python paket uyumluluğu.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple, PyBool};
use pyo3::exceptions::{PyAttributeError, PyKeyError, PyRuntimeError};
use comar_api::link::ComarLink;
use tokio::runtime::Runtime;
use std::sync::Arc;
use zvariant::{OwnedValue, Value};

#[pymodule]
fn _comar(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyLink>()?;
    m.add_class::<PyCall>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

// ── Tip dönüşümleri (zvariant <-> PyO3) ──────────────────────────────────────

pub fn python_to_dbus<'py>(py: Python<'py>, obj: &Bound<'py, PyAny>) -> PyResult<Vec<OwnedValue>> {
    if let Ok(tup) = obj.downcast::<PyTuple>() {
        let mut vals = Vec::new();
        for item in tup.iter() {
            let v = py_to_value(py, &item)?;
            let owned = v.try_to_owned().map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            vals.push(owned);
        }
        return Ok(vals);
    }
    let v = py_to_value(py, obj)?;
    let owned = v.try_to_owned().map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(vec![owned])
}

fn py_to_value<'v>(py: Python<'v>, obj: &Bound<'v, PyAny>) -> PyResult<Value<'v>> {
    if obj.is_none() {
        return Ok(Value::Str("".into()));
    }
    if let Ok(b) = obj.downcast::<PyBool>() {
        return Ok(Value::Bool(b.is_true()));
    }
    if let Ok(i) = obj.extract::<i64>() {
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
        let mut d = zvariant::Dict::new(
            zvariant::Signature::try_from("s").unwrap(),
            zvariant::Signature::try_from("v").unwrap(),
        );
        for (k, v) in dict.iter() {
            let dk = py_to_value(py, &k)?;
            let dv = py_to_value(py, &v)?;
            d.append(dk, Value::Value(Box::new(dv))).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        }
        return Ok(Value::Dict(d));
    }
    if let Ok(list) = obj.downcast::<PyList>() {
        let items: Vec<Value> = list.iter().map(|i| py_to_value(py, &i)).collect::<PyResult<_>>()?;
        let sig = items.first().map(|v| v.value_signature().to_owned())
            .unwrap_or_else(|| zvariant::Signature::from_static_str_unchecked("v"));
        let mut arr = zvariant::Array::new(sig);
        for item in items {
            arr.append(item).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        }
        return Ok(Value::Array(arr));
    }
    if let Ok(tup) = obj.downcast::<PyTuple>() {
        let fields: Vec<Value> = tup.iter().map(|i| py_to_value(py, &i)).collect::<PyResult<_>>()?;
        let mut sb = zvariant::StructureBuilder::new();
        for f in fields { sb.push_value(f); }
        return Ok(Value::Structure(sb.build()));
    }
    let s = obj.str()?.to_string();
    Ok(Value::Str(s.into()))
}

pub fn dbus_to_python<'py>(py: Python<'py>, vals: &[OwnedValue]) -> PyResult<Vec<Bound<'py, PyAny>>> {
    vals.iter().map(|v| value_to_py(py, &**v)).collect()
}

fn value_to_py<'py>(py: Python<'py>, val: &Value<'_>) -> PyResult<Bound<'py, PyAny>> {
    let obj: Bound<PyAny> = match val {
        Value::Bool(b)   => pyo3::types::PyBool::new_bound(py, *b).as_any().clone(),
        Value::I16(n)    => (*n as i64).to_object(py).into_bound(py).into_any(),
        Value::U16(n)    => (*n as i64).to_object(py).into_bound(py).into_any(),
        Value::I32(n)    => (*n as i64).to_object(py).into_bound(py).into_any(),
        Value::U32(n)    => (*n as i64).to_object(py).into_bound(py).into_any(),
        Value::I64(n)    => (*n).to_object(py).into_bound(py).into_any(),
        Value::U64(n)    => (*n as i64).to_object(py).into_bound(py).into_any(),
        Value::F64(f)    => (*f).to_object(py).into_bound(py).into_any(),
        Value::Str(s)    => s.as_str().to_object(py).into_bound(py).into_any(),
        Value::ObjectPath(p) => p.as_str().to_object(py).into_bound(py).into_any(),
        Value::Signature(s)  => s.as_str().to_object(py).into_bound(py).into_any(),

        Value::Array(arr) => {
            if arr.element_signature().starts_with('{') {
                let dict = PyDict::new_bound(py);
                for item in arr.iter() {
                    if let Value::Structure(kv) = item {
                        let fields: Vec<&Value> = kv.fields().iter().collect();
                        if fields.len() == 2 {
                            let k = value_to_py(py, fields[0])?;
                            let v = value_to_py(py, fields[1])?;
                            dict.set_item(k, v)?;
                        }
                    }
                }
                dict.into_any()
            } else {
                let list = PyList::new_bound(
                    py,
                    arr.iter().map(|v| value_to_py(py, v)).collect::<PyResult<Vec<_>>>()?,
                );
                list.into_any()
            }
        }
        Value::Dict(d) => {
            let dict = PyDict::new_bound(py);
            for (k, v) in d.iter() {
                let pk = value_to_py(py, &k)?;
                let pv = value_to_py(py, &v)?;
                dict.set_item(pk, pv)?;
            }
            dict.into_any()
        }
        Value::Structure(s) => {
            let items: Vec<Bound<PyAny>> = s.fields().iter().map(|f| value_to_py(py, f)).collect::<PyResult<_>>()?;
            PyTuple::new_bound(py, items).into_any()
        }
        Value::Value(inner) => value_to_py(py, inner)?,
        _ => py.None().into_bound(py),
    };
    Ok(obj)
}

// ── PyLink ────────────────────────────────────────────────────────────────────

#[pyclass(name = "Link")]
struct PyLink {
    address:   String,
    interface: String,
    rt: Arc<Runtime>,
    link: Arc<ComarLink>,
}

#[pymethods]
impl PyLink {
    #[new]
    #[pyo3(signature = (_version=None, socket=None, alternate=false))]
    fn new(_version: Option<&str>, socket: Option<&str>, alternate: bool) -> PyResult<Self> {
        let rt = Arc::new(Runtime::new().map_err(|e| PyRuntimeError::new_err(e.to_string()))?);
        
        let link = rt.block_on(async {
            let mut address = "tr.org.luppo.comar".to_string();
            if alternate { address.push('2'); }
            
            if let Some(s) = socket {
                ComarLink::new_with_socket(s).await
            } else {
                ComarLink::new(Some(&address)).await
            }
        }).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

        let address = link.address.clone();
        let interface = link.interface.clone();

        Ok(Self { interface, address, rt, link: Arc::new(link) })
    }

    #[allow(non_snake_case)]
    #[pyo3(signature = (locale=None))]
    fn setLocale(&self, locale: Option<&str>) -> PyResult<()> {
        if let Some(l) = locale {
            self.rt.block_on(self.link.set_locale(l))
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        }
        Ok(())
    }

    #[allow(non_snake_case)]
    #[pyo3(signature = (all=None))]
    fn listRunning(&self, all: Option<bool>) -> PyResult<Vec<String>> {
        let a = all.unwrap_or(true);
        self.rt.block_on(self.link.list_running(a))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }
    
    #[pyo3(signature = (_method=None))]
    fn cancel(&self, _method: Option<&str>) -> PyResult<i32> {
        self.rt.block_on(self.link.cancel())
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    fn register(&self, app: &str, model: &str, script: &str) -> PyResult<()> { 
        self.rt.block_on(self.link.register(app, model, script))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    fn remove(&self, app: &str) -> PyResult<()> { 
        self.rt.block_on(self.link.remove(app))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    #[allow(non_snake_case)]
    fn listenSignals(&self, py: Python<'_>, model: &str, handler: PyObject) -> PyResult<()> { 
        let handler_clone = handler.clone_ref(py);
        
        // Asenkron sinyal dinleme taskını rt üzerinde başlatıyoruz.
        // rt yaşadığı sürece çalışacaktır.
        self.rt.block_on(self.link.listen_signals(model, move |package, signal, args| {
            Python::with_gil(|py| {
                let py_args = match dbus_to_python(py, &args) {
                    Ok(a) => PyTuple::new_bound(py, a).into_any(),
                    Err(_) => PyTuple::empty_bound(py).into_any(),
                };
                let _ = handler_clone.call1(py, (package, signal, py_args));
            });
        })).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        
        Ok(())
    }

    fn __getattr__(&self, name: &str) -> PyResult<PyCall> {
        if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
            Ok(PyCall {
                rt: self.rt.clone(),
                link: self.link.clone(),
                address:   self.address.clone(),
                interface: self.interface.clone(),
                group:     name.to_string(),
                class:     None,
                package:   None,
                method:    None,
            })
        } else {
            Err(PyAttributeError::new_err(format!("'Link' nesnesinin '{}' özelliği yok", name)))
        }
    }
}

// ── PyCall ────────────────────────────────────────────────────────────────────

#[pyclass(name = "Call")]
#[derive(Clone)]
struct PyCall {
    rt: Arc<Runtime>,
    link: Arc<ComarLink>,
    #[allow(dead_code)]
    address:   String,
    #[allow(dead_code)]
    interface: String,
    group:     String,
    class:     Option<String>,
    package:   Option<String>,
    method:    Option<String>,
}

#[pymethods]
impl PyCall {
    fn __getattr__(&self, name: &str) -> PyResult<PyCall> {
        if self.class.is_none() {
            if name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                return Ok(PyCall { class: Some(name.to_string()), ..self.clone() });
            }
        } else {
            return Ok(PyCall { method: Some(name.to_string()), ..self.clone() });
        }
        Err(PyAttributeError::new_err(format!("Özellik bulunamadı: {}", name)))
    }

    fn __getitem__(&self, key: &str) -> PyResult<PyCall> {
        if self.class.is_none() {
            return Err(PyKeyError::new_err("Önce sınıf seçilmeli"));
        }
        Ok(PyCall { package: Some(key.replace('-', "_")), ..self.clone() })
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let apps = if let Some(ref cls) = self.class {
            let model = format!("{}.{}", self.group, cls);
            self.rt.block_on(self.link.list_model_applications(&model))
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
        } else {
            vec![]
        };
        let list = PyList::new_bound(py, apps);
        let builtins = py.import_bound("builtins")?;
        builtins.call_method1("iter", (list,))
    }

    #[pyo3(signature = (*args, **_kwargs))]
    fn __call__(
        &self,
        py:      Python<'_>,
        args:    &Bound<'_, PyTuple>,
        _kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyObject> {
        let app = self.package.as_deref().unwrap_or("");
        let cls = self.class.as_deref().unwrap_or("");
        let method = self.method.as_deref().unwrap_or("");
        
        if app.is_empty() {
            return Err(PyAttributeError::new_err("Package name required for call"));
        }
        
        let model = format!("{}.{}", self.group, cls);
        let dbus_args = python_to_dbus(py, &args.as_any())?;
        
        let mut sb = zvariant::StructureBuilder::new();
        for val in dbus_args {
            sb.push_value(val.into());
        }
        let body = sb.build();

        let reply = self.rt.block_on(self.link.call(app, &model, method, &body))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            
        let body_values: Vec<OwnedValue> = reply.body().deserialize().unwrap_or_default();
        let py_reply = dbus_to_python(py, &body_values)?;
        
        if py_reply.len() == 1 {
            Ok(py_reply[0].clone().into())
        } else if py_reply.is_empty() {
            Ok(py.None())
        } else {
            Ok(PyTuple::new_bound(py, py_reply).into_any().into())
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Call(group={}, class={:?}, package={:?}, method={:?})",
            self.group, self.class, self.package, self.method
        )
    }
}
