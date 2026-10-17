// Validation de schéma composée en constats (spec : render_lint_schema.sdd).
// `schema_known` et `schema_validate` sont fournis par le moteur (voir
// src/testing/schema/rhai.sdd) ; cette bibliothèque ne fait que traduire le pointeur JSON en
// notation pointée et choisir la sévérité. Aucune politique cachée : `strict`, `unknown` et
// `severity` sont des paramètres visibles de l'appel.

// Lit une clé d'une carte ; `()` si la valeur n'est pas une carte ou si la clé est absente.
private fn field(m, key) {
    if type_of(m) == "map" && key in m { m[key] } else { () }
}

// Chiffres ASCII uniquement, non vide : un segment d'indice de tableau.
private fn is_index(s) {
    if s == "" { return false; }
    for ch in s {
        if !("0123456789".contains(ch)) { return false; }
    }
    true
}

// Pointeur JSON (`/spec/template/spec/containers/0`) vers la notation pointée avec indices
// (`spec.template.spec.containers[0]`) ; chaîne vide à la racine.
private fn to_dotted(pointer) {
    let out = "";
    for seg in pointer.split("/") {
        if seg == "" { continue; }
        // `replace` modifie la chaîne sur place et ne rend rien dans Rhai.
        let s = seg;
        s.replace("~1", "/");
        s.replace("~0", "~");
        if is_index(s) {
            out += `[${s}]`;
        } else if out == "" {
            out = s;
        } else {
            out += "." + s;
        }
    }
    out
}

// Constats de schéma pour un objet.
fn check(obj) { check(obj, #{}) }

fn check(obj, opts) {
    import "render_lint" as render_lint;
    let strict = field(opts, "strict");
    if type_of(strict) != "bool" { strict = true; }
    let severity = field(opts, "severity");
    if type_of(severity) != "string" { severity = "error"; }
    let unknown = field(opts, "unknown");
    if type_of(unknown) != "string" { unknown = "warn"; }
    if unknown != "warn" && unknown != "error" && unknown != "ignore" {
        throw `schema: unknown doit valoir warn, error ou ignore, reçu "${unknown}"`;
    }
    let oref = render_lint::object_ref(obj);
    if oref.apiVersion == "" || oref.kind == "" {
        return [render_lint::finding("schema/invalid", severity, obj, "", "apiVersion et kind doivent être des chaînes non vides")];
    }
    if !schema_known(obj) {
        if unknown == "ignore" { return []; }
        return [render_lint::finding(
            "schema/unknown-kind",
            unknown,
            obj,
            `no schema for ${oref.apiVersion} ${oref.kind}`
        )];
    }
    let out = [];
    for e in schema_validate(obj, #{ strict: strict }) {
        out.push(render_lint::finding("schema/invalid", severity, obj, to_dotted(e.path), e.message));
    }
    out
}
