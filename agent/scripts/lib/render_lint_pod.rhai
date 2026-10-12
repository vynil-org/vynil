// Thème « pod » des vérifications portées de kube-linter (spec : render_lint_pod.sdd).
// Chaque vérification est pure : elle ne lit que ses arguments et retourne un tableau de
// constats, vide quand rien n'est à signaler. `render_lint` est importé dans le corps de chaque
// fonction (et non en tête) : le noyau importe ce module, un import en tête ferait un cycle.

// Lit une clé d'une carte ; `()` si la valeur n'est pas une carte ou si la clé est absente.
private fn field(m, key) {
    if type_of(m) == "map" && key in m { m[key] } else { () }
}

// Motifs de BlockList par défaut de kube-linter (`latest-tag`). Rhai n'a pas de moteur de regex :
// seuls ces trois motifs, à l'identique, sont reconnus. Tout autre motif lève une erreur explicite.
private fn blocked(image, block) {
    for pattern in block {
        if pattern == ".*:(latest)$" {
            if image.ends_with(":latest") { return true; }
        } else if pattern == "^[^:]*$" {
            if !image.contains(":") { return true; }
        } else if pattern == "(.*/[^:]+)$" {
            let parts = image.split("/");
            if parts.len() > 1 {
                let last = parts[parts.len() - 1];
                if last != "" && !last.contains(":") { return true; }
            }
        } else {
            throw `latest-tag : motif BlockList "${pattern}" non supporté (pas de regex dans Rhai ; seuls les motifs par défaut de kube-linter le sont)`;
        }
    }
    false
}

// Image d'un conteneur d'un objet qui utilise un tag `latest` ou pas de tag.
fn latest_tag(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let block = [".*:(latest)$", "^[^:]*$", "(.*/[^:]+)$"];
    if type_of(params) == "map" && "BlockList" in params && type_of(params.BlockList) == "array" {
        block = params.BlockList;
    }
    let findings = [];
    let kind = field(obj, "kind");
    let name = field(field(obj, "metadata"), "name");
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            let image = field(c.container, "image");
            if type_of(image) != "string" { continue; }
            if blocked(image, block) {
                findings.push(render_lint::finding(
                    "latest-tag",
                    "error",
                    obj,
                    c.path + ".image",
                    `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : image "${image}" sans tag fixe ; utiliser une version figée (ex. "app:1.2.3") ou un digest`
                ));
            }
        }
    }
    findings
}

// Conteneur (initialisation comprise) dont le système de fichiers racine n'est pas en lecture
// seule : `securityContext.readOnlyRootFilesystem` doit valoir `true`.
fn no_read_only_root_fs(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = field(obj, "kind");
    let name = field(field(obj, "metadata"), "name");
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            let ro = field(field(c.container, "securityContext"), "readOnlyRootFilesystem");
            if ro != true {
                findings.push(render_lint::finding(
                    "no-read-only-root-fs",
                    "error",
                    obj,
                    c.path,
                    `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : système de fichiers racine accessible en écriture ; définir securityContext.readOnlyRootFilesystem: true`
                ));
            }
        }
    }
    findings
}
