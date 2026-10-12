// Noyau de la bibliothèque render_lint (spec : render_lint.sdd).
// Constats, gabarits de pod, conteneurs, suppressions, et assemblage du catalogue de vérifications
// portées. `all` et `checks` s'assemblent par une liste explicite : Rhai ne ré-exporte pas les
// fonctions d'un module importé. Les modules thématiques sont importés dans le corps des fonctions
// (et non en tête) : ils importent ce noyau pour leurs constats, un import en tête ferait un cycle.

// Lit une clé d'une carte ; `()` si la valeur n'est pas une carte ou si la clé est absente.
private fn field(m, key) {
    if type_of(m) == "map" && key in m { m[key] } else { () }
}

// Chaîne, ou chaîne vide si la valeur n'en est pas une.
private fn text(v) {
    if type_of(v) == "string" { v } else { "" }
}

// Constat : rule, severity, object (référence), message ; `path` ajouté par la forme à 5 arguments.
fn finding(rule, severity, obj, message) {
    #{ rule: rule, severity: severity, object: object_ref(obj), message: message }
}

fn finding(rule, severity, obj, path, message) {
    #{ rule: rule, severity: severity, object: object_ref(obj), path: path, message: message }
}

// Référence d'un objet : apiVersion, kind, name, et namespace seulement s'il est présent.
fn object_ref(obj) {
    let meta = field(obj, "metadata");
    let oref = #{
        apiVersion: text(field(obj, "apiVersion")),
        kind: text(field(obj, "kind")),
        name: text(field(meta, "name"))
    };
    let ns = text(field(meta, "namespace"));
    if ns != "" { oref.namespace = ns; }
    oref
}

// Identité d'une référence, pour retrouver un objet de la liste à partir d'un constat.
private fn identity(oref) {
    `${oref.apiVersion}|${oref.kind}|${text(field(oref, "namespace"))}|${oref.name}`
}

// Gabarits de pod d'un objet : `#{ pod, path }`. `path` est `spec.template`, `spec.jobTemplate.spec.template`
// ou "" pour un Pod. Tableau vide pour un objet sans pod.
fn pod_templates(obj) {
    let kind = text(field(obj, "kind"));
    let spec = field(obj, "spec");
    let path = "";
    let template = ();
    if kind == "Pod" {
        if type_of(spec) != "map" { return []; }
        return [#{ pod: obj, path: "" }];
    } else if kind in ["Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "ReplicationController", "Job"] {
        path = "spec.template";
        template = field(spec, "template");
    } else if kind == "CronJob" {
        path = "spec.jobTemplate.spec.template";
        template = field(field(field(spec, "jobTemplate"), "spec"), "template");
    } else {
        return [];
    }
    if type_of(template) != "map" { return []; }
    [#{ pod: template, path: path }]
}

// Conteneurs d'une entrée de `pod_templates` : conteneurs puis conteneurs d'initialisation,
// `path` complet (`spec.template.spec.containers[0]`) et `init` booléen.
fn containers(entry) {
    let out = [];
    let pod = field(entry, "pod");
    let base = text(field(entry, "path"));
    let prefix = if base == "" { "spec" } else { base + ".spec" };
    let list = field(field(pod, "spec"), "containers");
    if type_of(list) == "array" {
        let i = 0;
        for c in list {
            out.push(#{ container: c, path: `${prefix}.containers[${i}]`, init: false });
            i += 1;
        }
    }
    let inits = field(field(pod, "spec"), "initContainers");
    if type_of(inits) == "array" {
        let i = 0;
        for c in inits {
            out.push(#{ container: c, path: `${prefix}.initContainers[${i}]`, init: true });
            i += 1;
        }
    }
    out
}

// Annotation de suppression `{ignore_prefix}{rule}` de l'objet, ou `()` si absente ou si
// `ignore_prefix` n'est pas donné.
fn suppression_reason(obj, rule, opts) {
    let prefix = field(opts, "ignore_prefix");
    if type_of(prefix) != "string" { return (); }
    let annotations = field(field(obj, "metadata"), "annotations");
    field(annotations, prefix + rule)
}

// Objet de la liste correspondant à la référence d'un constat, ou `()`.
private fn find_object(objs, oref) {
    let wanted = identity(oref);
    for o in objs {
        if identity(object_ref(o)) == wanted { return o; }
    }
    ()
}

// Marque les constats dont l'objet porte l'annotation de suppression. Sans `ignore_prefix`,
// rien n'est supprimé. Avec `require_reason`, une annotation vide ne supprime rien et ajoute un
// constat `suppression-without-reason` (une fois par objet et règle).
fn apply_suppressions(findings, objs, opts) {
    let prefix = field(opts, "ignore_prefix");
    if type_of(prefix) != "string" { return findings; }
    let require = field(opts, "require_reason") == true;
    let out = [];
    let extra = [];
    let seen = #{};
    for f in findings {
        let obj = find_object(objs, f.object);
        let value = suppression_reason(obj, f.rule, opts);
        if value == () {
            out.push(f);
            continue;
        }
        let reason = text(value);
        if reason == "" && require {
            out.push(f);
            let key = identity(f.object) + "|" + f.rule;
            if !(key in seen) {
                seen[key] = true;
                extra.push(finding(
                    "suppression-without-reason",
                    "error",
                    obj,
                    `annotation "${prefix}${f.rule}" vide : indiquer la raison de la suppression de "${f.rule}"`
                ));
            }
            continue;
        }
        let g = f;
        g.suppressed = true;
        if reason != "" { g.reason = reason; }
        g.origin = "annotation:" + prefix + f.rule;
        out.push(g);
    }
    for x in extra { out.push(x); }
    out
}

// Options normalisées : chaque clé présente, valeurs par défaut appliquées.
private fn settings(opts) {
    let exclude = field(opts, "exclude");
    if type_of(exclude) != "array" { exclude = []; }
    let severity = field(opts, "severity");
    if type_of(severity) != "map" { severity = #{}; }
    let params = field(opts, "params");
    if type_of(params) != "map" { params = #{}; }
    #{
        exclude: exclude,
        only: field(opts, "only"),
        severity: severity,
        params: params,
        ignore_prefix: field(opts, "ignore_prefix"),
        require_reason: field(opts, "require_reason") == true
    }
}

private fn enabled(cfg, name) {
    if name in cfg.exclude { return false; }
    if type_of(cfg.only) == "array" { return name in cfg.only; }
    true
}

// Paramètres d'une vérification : la carte donnée, ou une carte vide.
private fn param(cfg, name) {
    let p = field(cfg.params, name);
    if type_of(p) == "map" { p } else { #{} }
}

// Ajoute les constats d'une vérification à `out`, en appliquant la sévérité configurée.
private fn run_check(out, cfg, name, found) {
    let sev = field(cfg.severity, name);
    let result = out;
    for f in found {
        let g = f;
        if type_of(sev) == "string" { g.severity = sev; }
        result.push(g);
    }
    result
}

// Toutes les vérifications activées sur tous les objets, suppressions appliquées. Ordre : objets
// dans l'ordre, puis vérifications dans l'ordre alphabétique de leur nom.
fn all(objs, ctx) { all(objs, ctx, #{}) }

fn all(objs, ctx, opts) {
    import "render_lint_pod" as pod;
    // Les lots suivants ajoutent ici un `import` par thème et un appel par vérification, dans
    // l'ordre alphabétique global des noms (voir `checks`).
    let cfg = settings(opts);
    let raw = [];
    for obj in objs {
        if enabled(cfg, "latest-tag") {
            raw = run_check(raw, cfg, "latest-tag", pod::latest_tag(obj, objs, ctx, param(cfg, "latest-tag")));
        }
        if enabled(cfg, "no-read-only-root-fs") {
            raw = run_check(raw, cfg, "no-read-only-root-fs", pod::no_read_only_root_fs(obj, objs, ctx, param(cfg, "no-read-only-root-fs")));
        }
    }
    apply_suppressions(raw, objs, #{ ignore_prefix: cfg.ignore_prefix, require_reason: cfg.require_reason })
}

// Noms des vérifications portées, triés. Doit correspondre aux entrées `ported` du catalogue
// (fixtures/render_lint/catalogue.yaml) ; le test de cohérence le vérifie.
fn checks() {
    ["latest-tag", "no-read-only-root-fs"]
}
