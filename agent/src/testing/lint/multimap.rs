//! Index partagé offert à la bibliothèque de lint (spec : `multimap.sdd`).

use rhai::{Array, Dynamic, Engine, EvalAltResult, Position};
use std::{
    collections::HashMap,
    sync::{Arc, PoisonError, RwLock},
};

/// Nombre maximal de valeurs rangées dans une table, toutes clés confondues. Même valeur que la
/// borne de tableau du moteur de lint (`MAX_ARRAY_SIZE` de `runner.rs`) : la table est un tableau
/// déguisé et ne doit pas la contourner. Constante locale pour ne pas créer de dépendance
/// circulaire entre `runner` (qui enregistre ce module) et ce module.
const MAX_VALUES: usize = 1_000_000;

#[derive(Debug, Default)]
struct Table {
    entries: HashMap<String, Vec<Dynamic>>,
    /// Nombre de valeurs rangées, toutes clés confondues.
    total: usize,
    /// Borne de `total` : `MAX_VALUES` en production, une valeur plus petite dans les tests.
    limit: usize,
}

/// Table clé de chaîne -> valeurs, derrière un `Arc` : la copier (ce que fait Rhai à chaque
/// passage par valeur) copie la poignée, jamais le contenu.
#[derive(Clone, Debug)]
pub struct Multimap {
    table: Arc<RwLock<Table>>,
}

impl Default for Multimap {
    fn default() -> Self {
        Self::with_limit(MAX_VALUES)
    }
}

impl Multimap {
    fn with_limit(limit: usize) -> Self {
        Self {
            table: Arc::new(RwLock::new(Table {
                limit,
                ..Table::default()
            })),
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Table> {
        self.table.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn put(&self, key: &str, value: Dynamic) -> Result<(), Box<EvalAltResult>> {
        let mut table = self.table.write().unwrap_or_else(PoisonError::into_inner);
        if table.total >= table.limit {
            return Err(Box::new(EvalAltResult::ErrorRuntime(
                format!("multimap : borne de {} valeurs atteinte, put refusé", table.limit).into(),
                Position::NONE,
            )));
        }
        table.entries.entry(key.to_string()).or_default().push(value);
        table.total = table.total.saturating_add(1);
        drop(table);
        Ok(())
    }

    fn values(&self, key: &str) -> Array {
        self.read().entries.get(key).cloned().unwrap_or_default()
    }

    fn first(&self, key: &str) -> Dynamic {
        self.read()
            .entries
            .get(key)
            .and_then(|values| values.first().cloned())
            .unwrap_or(Dynamic::UNIT)
    }

    fn has(&self, key: &str) -> bool {
        self.read().entries.contains_key(key)
    }

    fn count(&self, key: &str) -> i64 {
        self.read()
            .entries
            .get(key)
            .map_or(0, |values| i64::try_from(values.len()).unwrap_or(i64::MAX))
    }

    fn at(&self, key: &str, index: i64) -> Dynamic {
        usize::try_from(index)
            .ok()
            .and_then(|index| {
                self.read()
                    .entries
                    .get(key)
                    .and_then(|values| values.get(index).cloned())
            })
            .unwrap_or(Dynamic::UNIT)
    }
}

/// Enregistre le type `Multimap` et ses fonctions dans `engine` : `multimap()`,
/// `put(key, value)`, `values(key)`, `first(key)`, `has(key)`, `count(key)`, `at(key, index)`.
pub fn register(engine: &mut Engine) {
    engine
        .register_type_with_name::<Multimap>("Multimap")
        .register_fn("multimap", || Multimap::with_limit(MAX_VALUES))
        .register_fn("put", |m: &mut Multimap, key: &str, value: Dynamic| {
            m.put(key, value)
        })
        .register_fn("values", |m: &mut Multimap, key: &str| m.values(key))
        .register_fn("first", |m: &mut Multimap, key: &str| m.first(key))
        .register_fn("has", |m: &mut Multimap, key: &str| m.has(key))
        .register_fn("count", |m: &mut Multimap, key: &str| m.count(key))
        .register_fn("at", |m: &mut Multimap, key: &str, index: i64| m.at(key, index));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhai::Scope;

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

    #[test]
    fn count_and_at_read_a_bucket_without_copying_it() {
        let e = engine();
        let script = r#"
            let m = multimap();
            m.put("ns", 10); m.put("ns", 20); m.put("ns", 30);
            [m.count("ns"), m.count("other"), m.at("ns", 1), m.at("ns", 3), m.at("other", 0), m.at("ns", -1)]
        "#;
        let got = e.eval::<Array>(script).expect("évaluation");
        assert_eq!(got[0].as_int().expect("entier"), 3);
        assert_eq!(got[1].as_int().expect("entier"), 0);
        assert_eq!(got[2].as_int().expect("entier"), 20);
        assert!(got[3].is_unit());
        assert!(got[4].is_unit());
        assert!(got[5].is_unit());
    }

    #[test]
    fn the_public_limit_is_one_million() {
        // Le `multimap()` que voit le script porte la borne de la spec, pas celle d'un test.
        let m = Multimap::default();
        for _ in 0..1_000_000 {
            m.put("k", Dynamic::UNIT).expect("sous la borne");
        }
        let err = m
            .put("k", Dynamic::UNIT)
            .expect_err("le millionième et unième put est refusé");
        assert!(err.to_string().contains("1000000"), "message: {err}");
        assert_eq!(m.read().total, 1_000_000);
    }

    #[test]
    fn put_is_bounded() {
        // Borne injectée à 3 pour le test ; le comportement public reste le million.
        let mut engine = Engine::new();
        register(&mut engine);
        let mut scope = Scope::new();
        scope.push("m", Multimap::with_limit(3));
        engine
            .eval_with_scope::<()>(&mut scope, r#"m.put("a", 1); m.put("a", 2); m.put("b", 3);"#)
            .expect("évaluation");
        let err = engine
            .eval_with_scope::<()>(&mut scope, r#"m.put("c", 4);"#)
            .expect_err("un quatrième put refusé est une erreur de script");
        assert!(err.to_string().contains('3'), "message: {err}");
        let got = engine
            .eval_with_scope::<Array>(&mut scope, r#"[m.count("a"), m.count("b"), m.count("c")]"#)
            .expect("évaluation");
        assert_eq!(got[0].as_int().expect("entier"), 2);
        assert_eq!(got[1].as_int().expect("entier"), 1);
        assert_eq!(got[2].as_int().expect("entier"), 0);
    }
}
