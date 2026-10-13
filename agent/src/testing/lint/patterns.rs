//! Fonction `regex_is_match` offerte au script de lint (spec : `patterns.sdd`).

use regex::{Regex, RegexBuilder};
use rhai::{Dynamic, Engine, EvalAltResult, Position};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock, PoisonError},
};

/// Taille maximale du programme compilé d'un motif (octets). Au-delà, le motif est refusé
/// plutôt que de consommer la mémoire du processus.
const COMPILED_SIZE_LIMIT: usize = 1 << 20;

/// Nombre maximal de motifs compilés conservés : au-delà, le cache est vidé. La bibliothèque
/// n'utilise que quelques dizaines de motifs, ce plafond ne borne que les cas pathologiques.
const CACHE_LIMIT: usize = 512;

/// Motifs déjà compilés. Un `Regex` se copie par poignée ; le cache ne change jamais le
/// résultat d'un appel, seulement son coût.
fn cache() -> &'static Mutex<HashMap<String, Regex>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn compiled(pattern: &str) -> Result<Regex, regex::Error> {
    if let Some(regex) = cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(pattern)
    {
        return Ok(regex.clone());
    }
    // Compilé hors du verrou : deux fils peuvent compiler le même motif, le résultat est identique.
    let regex = RegexBuilder::new(pattern)
        .size_limit(COMPILED_SIZE_LIMIT)
        .build()?;
    let mut cache = cache().lock().unwrap_or_else(PoisonError::into_inner);
    if cache.len() >= CACHE_LIMIT {
        cache.clear();
    }
    cache.insert(pattern.to_string(), regex.clone());
    drop(cache);
    Ok(regex)
}

/// Enregistre `regex_is_match(pattern, text) -> bool` dans `engine`.
pub fn register(engine: &mut Engine) {
    engine.register_fn("regex_is_match", regex_is_match);
}

/// Vrai quand `pattern` trouve une correspondance dans `text`. Motif invalide ou trop grand :
/// erreur d'exécution Rhai, jamais un panic.
fn regex_is_match(pattern: &str, text: &str) -> Result<bool, Box<EvalAltResult>> {
    let regex = compiled(pattern).map_err(|e| {
        Box::new(EvalAltResult::ErrorRuntime(
            Dynamic::from(format!("regex_is_match: invalid pattern {pattern}: {e}")),
            Position::NONE,
        ))
    })?;
    Ok(regex.is_match(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        let mut engine = Engine::new();
        register(&mut engine);
        engine
    }

    fn eval(script: &str) -> Result<bool, Box<rhai::EvalAltResult>> {
        engine().eval::<bool>(script)
    }

    #[test]
    fn an_unanchored_pattern_matches_inside_the_text() {
        let found = eval(r#"regex_is_match("latest", "registry/app:latest")"#);
        assert!(found.expect("évaluation"));
    }

    #[test]
    fn the_cache_never_changes_a_result_and_stays_bounded() {
        let e = engine();
        for i in 0..(CACHE_LIMIT * 2) {
            let hit = e
                .eval::<bool>(&format!(r#"regex_is_match("^p{i}$", "p{i}")"#))
                .expect("évaluation");
            assert!(hit);
            let miss = e
                .eval::<bool>(&format!(r#"regex_is_match("^p{i}$", "q{i}")"#))
                .expect("évaluation");
            assert!(!miss);
        }
        let size = cache().lock().unwrap_or_else(PoisonError::into_inner).len();
        assert!(size <= CACHE_LIMIT, "{size}");
    }

    #[test]
    fn anchors_constrain_the_match() {
        let on_app = eval(r#"regex_is_match("^app$", "app")"#);
        let on_path = eval(r#"regex_is_match("^app$", "registry/app")"#);
        assert!(on_app.expect("évaluation"));
        assert!(!on_path.expect("évaluation"));
    }

    #[test]
    fn the_default_latest_tag_block_list() {
        let script = r#"
            fn hit(text) {
                let block = [".*:(latest)$", "^[^:]*$", "(.*/[^:]+)$"];
                for p in block { if regex_is_match(p, text) { return true; } }
                false
            }
            [hit("nginx:latest"), hit("nginx"), hit("nginx:1.27")]
        "#;
        let hits = engine().eval::<rhai::Array>(script).expect("évaluation");
        let flags: Vec<bool> = hits.into_iter().map(|d| d.as_bool().expect("booléen")).collect();
        assert_eq!(flags, vec![true, true, false]);
    }

    #[test]
    fn an_invalid_pattern_is_a_script_error() {
        let err = eval(r#"regex_is_match("(", "x")"#).expect_err("erreur attendue");
        let rhai::EvalAltResult::ErrorRuntime(payload, _) = *err else {
            panic!("erreur d'exécution attendue");
        };
        let message = payload.to_string();
        assert!(
            message.starts_with("regex_is_match: invalid pattern"),
            "message inattendu : {message}"
        );
    }

    #[test]
    fn a_pathological_pattern_stays_linear() {
        let text = format!("{}b", "a".repeat(50));
        let script = format!(r#"regex_is_match("(a+)+$", "{text}")"#);
        assert!(!eval(&script).expect("évaluation"));
    }
}
