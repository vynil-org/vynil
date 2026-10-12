//! Garde de la règle de `/agent/scripts/lib/yaml_docs.sdd` : seul `yaml_docs.rhai` appelle
//! `yaml_decode_multi`. Depuis vynil-core 0.8.0 il ne filtre plus les documents vides, tout
//! autre script passe par `yd::decode`.

use std::{
    fs,
    path::{Path, PathBuf},
};

fn rhai_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rhai_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rhai") {
            out.push(path);
        }
    }
}

/// Retire les commentaires `/* ... */` et `// ...` : un appel commenté n'est pas un appel.
fn strip_comments(src: &str) -> String {
    let mut out = String::new();
    let mut rest = src;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start..]
            .find("*/")
            .map_or("", |end| &rest[start + end + 2..]);
    }
    out.push_str(rest);
    out.lines()
        .map(|l| l.find("//").map_or(l, |i| &l[..i]))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn no_script_calls_yaml_decode_multi_directly() {
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let mut files = vec![];
    rhai_files(&scripts, &mut files);
    assert!(
        !files.is_empty(),
        "aucun script trouvé sous {}",
        scripts.display()
    );
    let offenders: Vec<String> = files
        .iter()
        .filter(|f| !f.ends_with("lib/yaml_docs.rhai"))
        .filter(|f| strip_comments(&fs::read_to_string(f).unwrap()).contains("yaml_decode_multi("))
        .map(|f| f.strip_prefix(&scripts).unwrap().display().to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "appel direct à yaml_decode_multi (utiliser yd::decode de yaml_docs) : {offenders:?}"
    );
}
