use crate::{Error, RhaiRes, rhai_err};
use indexmap::IndexMap;
use rhai::{Dynamic, Engine, ImmutableString};
use rust_yaml::{Value, Yaml, YamlConfig, yaml::IndentConfig};
use std::str::FromStr;

// ── Re-exports from core (serde helpers) ──────────────────────────────────────

pub use vynil_core::yaml::{yaml_all_serialize_to_string, yaml_serialize_to_string, yaml_str_to_json};

fn new_yaml() -> Yaml {
    Yaml::with_config(YamlConfig {
        preserve_comments: true,
        loader_type: rust_yaml::LoaderType::RoundTrip,
        emit_anchors: false,
        indent: IndentConfig {
            indent: 2,
            sequence_indent: Some(0),
            ..Default::default()
        },
        ..Default::default()
    })
}

// ── Order-preserving YAML document type (rust-yaml) ───────────────────────────

/// A YAML document that preserves key insertion order (backed by `IndexMap`).
#[derive(Debug, Clone)]
pub struct YamlDoc(pub Value);

impl std::str::FromStr for YamlDoc {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        new_yaml().load_str(s).map(YamlDoc).map_err(|e| e.to_string())
    }
}

impl YamlDoc {
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or the YAML cannot be deserialized.
    pub fn to_yaml_string(&self) -> Result<String, String> {
        new_yaml().dump_str(&self.0).map_err(|e| e.to_string())
    }

    // ── Rhai indexer get ──────────────────────────────────────────────────
    pub fn idx_get(&mut self, key: ImmutableString) -> Dynamic {
        let key: String = key.into();
        match &self.0 {
            Value::Mapping(m) => {
                let k = Value::String(key);
                m.get(&k).map_or(Dynamic::UNIT, |v| value_to_dynamic(v.clone()))
            }
            Value::Sequence(s) => key
                .parse::<usize>()
                .ok()
                .and_then(|i| s.get(i))
                .map_or(Dynamic::UNIT, |v| value_to_dynamic(v.clone())),
            _ => Dynamic::UNIT,
        }
    }

    // ── Rhai indexer set ──────────────────────────────────────────────────
    pub fn idx_set(&mut self, key: ImmutableString, val: Dynamic) {
        let key: String = key.into();
        let yaml_val = dynamic_to_value(val);
        if let Value::Mapping(m) = &mut self.0 {
            m.insert(Value::String(key), yaml_val);
        }
    }

    // ── Rhai methods ──────────────────────────────────────────────────────

    pub fn keys(&mut self) -> rhai::Array {
        match &self.0 {
            Value::Mapping(m) => m
                .keys()
                .filter_map(|k| {
                    if let Value::String(s) = k {
                        Some(Dynamic::from(ImmutableString::from(s.as_str())))
                    } else {
                        None
                    }
                })
                .collect(),
            _ => rhai::Array::new(),
        }
    }

    pub fn values(&mut self) -> rhai::Array {
        match &self.0 {
            Value::Mapping(m) => m.values().map(|v| value_to_dynamic(v.clone())).collect(),
            _ => rhai::Array::new(),
        }
    }

    /// Retire `key` d'une racine mapping. Retrait par décalage : les autres clés gardent leur
    /// ordre. Rend `false` (document inchangé) si la clé est absente ou si la racine n'est
    /// pas un mapping.
    pub fn remove(&mut self, key: ImmutableString) -> bool {
        let key: String = key.into();
        match &mut self.0 {
            Value::Mapping(m) => m.shift_remove(&Value::String(key)).is_some(),
            _ => false,
        }
    }

    #[must_use]
    pub fn contains_key(&self, key: ImmutableString) -> bool {
        let key: String = key.into();
        match &self.0 {
            Value::Mapping(m) => m.contains_key(&Value::String(key)),
            _ => false,
        }
    }

    #[must_use]
    pub fn len(&self) -> i64 {
        match &self.0 {
            Value::Mapping(m) => i64::try_from(m.len()).unwrap_or(i64::MAX),
            Value::Sequence(s) => i64::try_from(s.len()).unwrap_or(i64::MAX),
            _ => 0,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ── Conversions: rust_yaml::Value ↔ Rhai Dynamic ─────────────────────────────

pub fn value_to_dynamic(v: Value) -> Dynamic {
    match v {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => Dynamic::from(b),
        Value::Int(i) => Dynamic::from(i),
        Value::Float(f) => Dynamic::from(f),
        Value::String(s) => Dynamic::from(ImmutableString::from(s.as_str())),
        Value::Sequence(a) => Dynamic::from_array(a.into_iter().map(value_to_dynamic).collect()),
        Value::Mapping(_) => Dynamic::from(YamlDoc(v)),
    }
}

pub fn dynamic_to_value(d: Dynamic) -> Value {
    if d.is_unit() {
        Value::Null
    } else if d.is::<YamlDoc>() {
        d.cast::<YamlDoc>().0
    } else if d.is::<bool>() {
        Value::Bool(d.cast::<bool>())
    } else if d.is::<i64>() {
        Value::Int(d.cast::<i64>())
    } else if d.is::<f64>() {
        Value::Float(d.cast::<f64>())
    } else if d.is::<ImmutableString>() {
        Value::String(d.cast::<ImmutableString>().to_string())
    } else if d.is::<String>() {
        Value::String(d.cast::<String>())
    } else if d.is::<rhai::Array>() {
        let arr = d.cast::<rhai::Array>();
        Value::Sequence(arr.into_iter().map(dynamic_to_value).collect())
    } else if d.is::<rhai::Map>() {
        let map = d.cast::<rhai::Map>();
        let indexmap: IndexMap<Value, Value> = map
            .into_iter()
            .map(|(k, v)| (Value::String(k.to_string()), dynamic_to_value(v)))
            .collect();
        Value::Mapping(indexmap)
    } else {
        serde_json::to_string(&d)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .map_or(Value::Null, serde_json_value_to_yaml_value)
    }
}

fn serde_json_value_to_yaml_value(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Value::Int(i),
            (None, Some(f)) => Value::Float(f),
            _ => Value::String(n.to_string()),
        },
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(a) => {
            Value::Sequence(a.into_iter().map(serde_json_value_to_yaml_value).collect())
        }
        serde_json::Value::Object(m) => {
            let indexmap: IndexMap<Value, Value> = m
                .into_iter()
                .map(|(k, v)| (Value::String(k), serde_json_value_to_yaml_value(v)))
                .collect();
            Value::Mapping(indexmap)
        }
    }
}

// ── Rhai registration (ordered / YamlDoc only) ────────────────────────────────

pub fn yaml_ordered_rhai_register(engine: &mut Engine) {
    engine
        .register_fn(
            "yaml_decode_ordered",
            |val: ImmutableString| -> RhaiRes<YamlDoc> {
                YamlDoc::from_str(val.as_ref()).map_err(|e| rhai_err(Error::YamlError(e)))
            },
        )
        .register_fn(
            "yaml_encode_ordered",
            |val: Dynamic| -> RhaiRes<ImmutableString> {
                let yaml_val = dynamic_to_value(val);
                new_yaml()
                    .dump_str(&yaml_val)
                    .map_err(|e| rhai_err(Error::YamlError(e.to_string())))
                    .map(std::convert::Into::into)
            },
        )
        .register_fn("yaml_encode_ordered", |yd: YamlDoc| -> RhaiRes<ImmutableString> {
            new_yaml()
                .dump_str(&yd.0)
                .map_err(|e| rhai_err(Error::YamlError(e.to_string())))
                .map(std::convert::Into::into)
        });

    engine
        .register_type_with_name::<YamlDoc>("map")
        .register_indexer_get(YamlDoc::idx_get)
        .register_indexer_set(YamlDoc::idx_set)
        .register_fn("keys", YamlDoc::keys)
        .register_fn("values", YamlDoc::values)
        .register_fn("len", YamlDoc::len)
        .register_fn("is_empty", YamlDoc::is_empty)
        .register_fn("contains", YamlDoc::contains_key)
        .register_fn("remove", YamlDoc::remove)
        .register_fn("to_string", |yd: &mut YamlDoc| -> String {
            yd.to_yaml_string().unwrap_or_default()
        });
}
