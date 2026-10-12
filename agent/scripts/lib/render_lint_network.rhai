// Thème « network » des vérifications portées de kube-linter (spec : render_lint_network.sdd).
// Chaque vérification est pure : elle ne lit que ses arguments et retourne un tableau de
// constats, vide quand rien n'est à signaler. `render_lint` est importé dans le corps de chaque
// fonction (et non en tête) : le noyau importe ce module, un import en tête ferait un cycle.
//
// Périmètre des genres : chaque vérification a son propre ensemble de groupes d'API, mesuré sur
// kube-linter v0.8.3 (le groupe n'est donc pas une constante du thème). Un objet mal formé pour
// son genre (décodage typé de l'outil) est ignoré en entier : aucun constat, et il n'est pas
// retenu comme cible d'un autre objet.

// Lit une clé d'une carte ; `()` si la valeur n'est pas une carte ou si la clé est absente.
private fn field(m, key) {
    if type_of(m) == "map" && key in m { m[key] } else { () }
}

// Chaîne, ou chaîne vide si la valeur n'en est pas une.
private fn text(v) {
    if type_of(v) == "string" { v } else { "" }
}

// Vrai quand l'objet a l'un des genres `kinds` dans le groupe `group` (noyau : "").
private fn kind_of_group(obj, kinds, group) {
    import "render_lint" as render_lint;
    render_lint::kind_of_group(obj, kinds, group)
}

// Namespace d'un objet (chaîne vide si absent) : comparé tel quel, `""` égal à `""` (mesuré).
private fn ns_of(obj) {
    text(field(field(obj, "metadata"), "namespace"))
}

// Nom d'un objet (chaîne vide si absent).
private fn name_of(obj) {
    text(field(field(obj, "metadata"), "name"))
}

// Labels d'une métadonnée, ou `()` si absents ou si une valeur n'est pas une chaîne (un labels
// non conforme rend l'objet non décodable par l'outil, donc non sélectionnable).
private fn labels_of(meta) {
    let labels = field(meta, "labels");
    if type_of(labels) != "map" { return (); }
    for k in labels.keys() {
        if type_of(labels[k]) != "string" { return (); }
    }
    labels
}

// Labels d'un gabarit de pod (`template`), ou `()`.
private fn template_labels(template) {
    if type_of(template) != "map" { return (); }
    labels_of(field(template, "metadata"))
}

// Labels du gabarit de pod qu'un Service peut sélectionner (mesuré sur kube-linter v0.8.3) :
//   Pod (noyau) : metadata.labels ;
//   Deployment, ReplicaSet, DaemonSet (apps ou extensions), StatefulSet (apps), Job (batch),
//   ReplicationController (noyau) : spec.template.metadata.labels ;
//   CronJob (batch) : spec.jobTemplate.spec.template.metadata.labels.
// `()` pour tout autre objet.
private fn selectable_labels(obj) {
    let spec = field(obj, "spec");
    if kind_of_group(obj, ["Pod"], "") {
        labels_of(field(obj, "metadata"))
    } else if kind_of_group(obj, ["Deployment", "ReplicaSet", "DaemonSet"], "apps")
        || kind_of_group(obj, ["Deployment", "ReplicaSet", "DaemonSet"], "extensions")
        || kind_of_group(obj, ["StatefulSet"], "apps")
        || kind_of_group(obj, ["Job"], "batch")
        || kind_of_group(obj, ["ReplicationController"], "") {
        template_labels(field(spec, "template"))
    } else if kind_of_group(obj, ["CronJob"], "batch") {
        template_labels(field(field(field(spec, "jobTemplate"), "spec"), "template"))
    } else {
        ()
    }
}

// Vrai quand tous les couples du sélecteur figurent dans les labels. Sélecteur vide : jamais
// (mesuré : un Service à sélecteur vide est dangling, il ne correspond à aucun gabarit).
private fn selects(sel, labels) {
    if type_of(labels) != "map" || sel.len() == 0 { return false; }
    for k in sel.keys() {
        if !(k in labels) || labels[k] != sel[k] { return false; }
    }
    true
}

// Vrai quand le Service se décode : spec absent ou carte, selector absent ou carte de chaînes,
// type absent ou chaîne, ports absents ou tableau de cartes dont `port` est entier, `name` une
// chaîne et `targetPort` un entier ou une chaîne. Sinon l'outil ignore le Service (mesuré).
private fn service_well_formed(svc) {
    let spec = field(svc, "spec");
    if spec == () { return true; }
    if type_of(spec) != "map" { return false; }
    let sel = field(spec, "selector");
    if sel != () {
        if type_of(sel) != "map" { return false; }
        for k in sel.keys() {
            if type_of(sel[k]) != "string" { return false; }
        }
    }
    let ty = field(spec, "type");
    if ty != () && type_of(ty) != "string" { return false; }
    let ports = field(spec, "ports");
    if ports != () {
        if type_of(ports) != "array" { return false; }
        for p in ports {
            if type_of(p) != "map" { return false; }
            let port = field(p, "port");
            if port != () && type_of(port) != "i64" { return false; }
            let pname = field(p, "name");
            if pname != () && type_of(pname) != "string" { return false; }
            let target = field(p, "targetPort");
            if target != () && type_of(target) != "i64" && type_of(target) != "string" { return false; }
        }
    }
    true
}

// Service dangling : aucun gabarit de pod du même namespace n'est sélectionné par son
// `spec.selector`. Un Service `ExternalName` (casse exacte, mesurée) est exempté, sans sélecteur
// ou non. Un Service mal formé : pas de constat.
fn dangling_service(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    if !kind_of_group(obj, ["Service"], "") { return []; }
    if !service_well_formed(obj) { return []; }
    let spec = field(obj, "spec");
    if text(field(spec, "type")) == "ExternalName" { return []; }
    let sel = field(spec, "selector");
    let ns = ns_of(obj);
    if type_of(sel) == "map" {
        for o in objs {
            if ns_of(o) == ns && selects(sel, selectable_labels(o)) { return []; }
        }
    }
    let name = name_of(obj);
    [render_lint::finding(
        "dangling-service",
        "error",
        obj,
        "spec.selector",
        `Service "${name}" : spec.selector ne correspond à aucun gabarit de pod du namespace "${ns}" ; corriger le sélecteur ou créer la charge de travail visée`
    )]
}

// Types de Service interdits : le paramètre `forbiddenServiceTypes` (défaut NodePort, LoadBalancer).
// Un paramètre présent mais qui n'est pas un tableau : pas de constat (l'outil refuse de démarrer).
private fn forbidden_types(params) {
    let v = field(params, "forbiddenServiceTypes");
    if v == () { ["NodePort", "LoadBalancer"] } else { v }
}

// Service exposé par un type interdit. La comparaison ignore la casse (mesuré : `nodeport` est
// signalé comme `NodePort`). Type absent ou vide : jamais signalé.
fn exposed_services(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    if !kind_of_group(obj, ["Service"], "") { return []; }
    if !service_well_formed(obj) { return []; }
    let forbidden = forbidden_types(params);
    if type_of(forbidden) != "array" { return []; }
    let ty = text(field(field(obj, "spec"), "type"));
    if ty == "" { return []; }
    let hit = false;
    for f in forbidden {
        if type_of(f) == "string" && f.to_upper() == ty.to_upper() { hit = true; }
    }
    if !hit { return []; }
    let name = name_of(obj);
    [render_lint::finding(
        "exposed-services",
        "error",
        obj,
        "spec.type",
        `Service "${name}" : type ${ty} interdit (exposition hors du cluster) ; utiliser ClusterIP et un Ingress, ou retirer le type ${ty}`
    )]
}
