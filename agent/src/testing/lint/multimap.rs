//! Index partagé offert à la bibliothèque de lint (spec : `multimap.sdd`).

use rhai::{Array, Dynamic, Engine};
use std::{
    collections::HashMap,
    sync::{Arc, PoisonError, RwLock},
};

/// Table clé de chaîne -> valeurs, derrière un `Arc` : la copier (ce que fait Rhai à chaque
/// passage par valeur) copie la poignée, jamais le contenu.
#[derive(Clone, Debug, Default)]
pub struct Multimap {
    entries: Arc<RwLock<HashMap<String, Vec<Dynamic>>>>,
}

impl Multimap {
    fn put(&self, key: &str, value: Dynamic) {
        self.entries
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(key.to_string())
            .or_default()
            .push(value);
    }

    fn values(&self, key: &str) -> Array {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    fn first(&self, key: &str) -> Dynamic {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .and_then(|values| values.first().cloned())
            .unwrap_or(Dynamic::UNIT)
    }

    fn has(&self, key: &str) -> bool {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(key)
    }
}

/// Enregistre le type `Multimap` et ses fonctions dans `engine` : `multimap()`,
/// `put(key, value)`, `values(key)`, `first(key)`, `has(key)`.
pub fn register(engine: &mut Engine) {
    engine
        .register_type_with_name::<Multimap>("Multimap")
        .register_fn("multimap", Multimap::default)
        .register_fn("put", |m: &mut Multimap, key: &str, value: Dynamic| {
            m.put(key, value);
        })
        .register_fn("values", |m: &mut Multimap, key: &str| m.values(key))
        .register_fn("first", |m: &mut Multimap, key: &str| m.first(key))
        .register_fn("has", |m: &mut Multimap, key: &str| m.has(key));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        let mut engine = Engine::new();
        register(&mut engine);
        engine
    }

    #[test]
    fn values_come_back_in_insertion_order() {
        let got = engine()
            .eval::<Array>(
                r#"let m = multimap(); m.put("ns", 1); m.put("ns", 2); m.put("other", 9); m.values("ns")"#,
            )
            .expect("évaluation");
        let ints: Vec<i64> = got.into_iter().map(|d| d.as_int().expect("entier")).collect();
        assert_eq!(ints, vec![1, 2]);
    }

    #[test]
    fn an_absent_key_is_empty_and_first_is_unit() {
        let e = engine();
        let values = e
            .eval::<Array>(r#"let m = multimap(); m.values("nope")"#)
            .expect("évaluation");
        assert_eq!(values.len(), 0);
        let first = e
            .eval::<Dynamic>(r#"let m = multimap(); m.first("nope")"#)
            .expect("évaluation");
        assert!(first.is_unit());
        assert!(
            !e.eval::<bool>(r#"let m = multimap(); m.has("nope")"#)
                .expect("évaluation")
        );
    }

    #[test]
    fn a_copy_shares_the_content() {
        // Passer la poignée par valeur à une fonction voit les mêmes entrées : c'est ce qui
        // évite de recopier l'index à chaque appel de vérification.
        let got = engine()
            .eval::<bool>(
                r#"
                fn seen(m) { m.has("k") }
                let m = multimap();
                m.put("k", #{ a: 1 });
                seen(m)
                "#,
            )
            .expect("évaluation");
        assert!(got);
    }

    #[test]
    fn the_handle_survives_a_map_copy() {
        let got = engine()
            .eval::<i64>(
                r#"
                let idx = #{ targets: multimap() };
                idx.targets.put("ns", 41);
                let copy = idx;
                copy.targets.put("ns", 1);
                idx.targets.values("ns").len()
                "#,
            )
            .expect("évaluation");
        assert_eq!(got, 2);
    }
}
