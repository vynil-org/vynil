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

// Problèmes d'un nom de port au sens IANA (mesuré sur kube-linter v0.8.3) : un message par règle
// violée, donc plusieurs constats possibles pour un même nom. Chaîne vide : deux règles.
private fn iana_name_problems(s) {
    let out = [];
    if s.len() > 15 { out.push("doit compter au plus 15 caractères"); }
    if !regex_is_match("^[a-z0-9-]+$", s) { out.push("ne doit contenir que des lettres a-z, des chiffres et des tirets"); }
    if !regex_is_match("[a-z]", s) { out.push("doit contenir au moins une lettre a-z"); }
    if s.starts_with("-") || s.ends_with("-") { out.push("ne doit pas commencer ni finir par un tiret"); }
    if s.contains("--") { out.push("ne doit pas contenir deux tirets consécutifs"); }
    out
}

// Vrai quand les ports d'un conteneur se décodent : absents, ou tableau de cartes dont `name` est
// une chaîne et `containerPort` un entier. Sinon l'outil ignore tout le gabarit (mesuré).
private fn container_ports_well_formed(c) {
    let ports = field(c, "ports");
    if ports == () { return true; }
    if type_of(ports) != "array" { return false; }
    for p in ports {
        if type_of(p) != "map" { return false; }
        let n = field(p, "name");
        if n != () && type_of(n) != "string" { return false; }
        let cp = field(p, "containerPort");
        if cp != () && type_of(cp) != "i64" { return false; }
    }
    true
}

// Vérification invalid-target-ports : nom IANA invalide sur (a) le `targetPort` chaîne d'un
// Service du noyau, et (b) le nom d'un port de conteneur ou d'init-conteneur d'un gabarit de pod
// (Pod, apps toutes versions, batch, ReplicationController ; le groupe extensions est ignoré,
// mesuré). Un nom de port de conteneur vide n'est pas examiné. Un constat par règle violée.
fn invalid_target_ports(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let out = [];
    if kind_of_group(obj, ["Service"], "") {
        if !service_well_formed(obj) { return []; }
        let ports = field(field(obj, "spec"), "ports");
        if type_of(ports) != "array" { return []; }
        let name = name_of(obj);
        let i = 0;
        for p in ports {
            let target = field(p, "targetPort");
            if type_of(target) == "string" {
                for why in iana_name_problems(target) {
                    out.push(render_lint::finding(
                        "invalid-target-ports",
                        "error",
                        obj,
                        `spec.ports[${i}].targetPort`,
                        `Service "${name}" : targetPort "${target}" ${why} ; corriger le nom du port cible (format IANA)`
                    ));
                }
            }
            i += 1;
        }
        return out;
    }
    let entries = render_lint::pod_templates(obj);
    for entry in entries {
        let containers = render_lint::containers(entry);
        for c in containers {
            if !container_ports_well_formed(c.container) { return []; }
        }
    }
    for entry in entries {
        for c in render_lint::containers(entry) {
            let cname = text(field(c.container, "name"));
            let ports = field(c.container, "ports");
            if type_of(ports) != "array" { continue; }
            let j = 0;
            for p in ports {
                let pname = text(field(p, "name"));
                if pname != "" {
                    for why in iana_name_problems(pname) {
                        out.push(render_lint::finding(
                            "invalid-target-ports",
                            "error",
                            obj,
                            `${c.path}.ports[${j}].name`,
                            `port "${pname}" du conteneur "${cname}" ${why} ; corriger le nom du port (format IANA)`
                        ));
                    }
                }
                j += 1;
            }
        }
    }
    out
}

// Backend d'une référence d'Ingress : `[#{ service, path }]` pour un backend `service`, `[]` pour
// un backend absent ou sans `service` (ressource), `()` si la forme est inattendue.
private fn backend_of(backend, path) {
    if backend == () { return []; }
    if type_of(backend) != "map" { return (); }
    let svc = field(backend, "service");
    if svc == () { return []; }
    if type_of(svc) != "map" { return (); }
    let name = field(svc, "name");
    if name != () && type_of(name) != "string" { return (); }
    let port = field(svc, "port");
    if port != () {
        if type_of(port) != "map" { return (); }
        let pn = field(port, "name");
        if pn != () && type_of(pn) != "string" { return (); }
        let num = field(port, "number");
        if num != () && type_of(num) != "i64" { return (); }
    }
    [#{ service: svc, path: path }]
}

// Backends d'un Ingress (defaultBackend puis chaque chemin de chaque règle) : `()` si l'Ingress
// est mal formé à un endroit quelconque. L'outil ignore alors tout l'objet (mesuré).
private fn ingress_backends(obj) {
    let spec = field(obj, "spec");
    if spec == () { return []; }
    if type_of(spec) != "map" { return (); }
    let out = [];
    let def = backend_of(field(spec, "defaultBackend"), "spec.defaultBackend.service");
    if def == () { return (); }
    for b in def { out.push(b); }
    let rules = field(spec, "rules");
    if rules == () { return out; }
    if type_of(rules) != "array" { return (); }
    let i = 0;
    for r in rules {
        if type_of(r) != "map" { return (); }
        let http = field(r, "http");
        if http != () {
            if type_of(http) != "map" { return (); }
            let paths = field(http, "paths");
            if paths != () {
                if type_of(paths) != "array" { return (); }
                let j = 0;
                for p in paths {
                    if type_of(p) != "map" { return (); }
                    let path = `spec.rules[${i}].http.paths[${j}].backend.service`;
                    let got = backend_of(field(p, "backend"), path);
                    if got == () { return (); }
                    for b in got { out.push(b); }
                    j += 1;
                }
            }
        }
        i += 1;
    }
    out
}

// Vrai quand un Service du noyau, de même namespace et de même nom, porte un port qui satisfait
// le backend. Un nom de port non vide se compare au `name` du port ; sinon le numéro (0 si absent)
// se compare au `port` du Service. Mesuré : un port absent du backend compte comme 0, et
// un nom présent prime sur le numéro. Un Service mal formé ne compte pas.
private fn backend_resolves(objs, ns, svc_name, port) {
    let pname = text(field(port, "name"));
    let pnum = field(port, "number");
    if pnum == () { pnum = 0; }
    for o in objs {
        if !kind_of_group(o, ["Service"], "") || ns_of(o) != ns || name_of(o) != svc_name { continue; }
        if !service_well_formed(o) { continue; }
        let ports = field(field(o, "spec"), "ports");
        if type_of(ports) != "array" { continue; }
        for p in ports {
            if pname != "" {
                if text(field(p, "name")) == pname { return true; }
            } else {
                let sp = field(p, "port");
                if type_of(sp) == "i64" && sp == pnum { return true; }
            }
        }
    }
    false
}

// Ingress dont un backend `service` ne résout aucun Service de la liste (même namespace, même
// nom, ou port absent du Service). Seul networking.k8s.io/v1 est examiné (mesuré : les Ingress
// extensions/v1beta1 et networking.k8s.io/v1beta1, au format legacy ou moderne, sont ignorés).
// Un constat par backend non résolu : defaultBackend puis chaque chemin de chaque règle.
fn dangling_ingress(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    if text(field(obj, "apiVersion")) != "networking.k8s.io/v1" { return []; }
    if text(field(obj, "kind")) != "Ingress" { return []; }
    let backends = ingress_backends(obj);
    if backends == () { return []; }
    let ns = ns_of(obj);
    let name = name_of(obj);
    let out = [];
    for b in backends {
        let svc_name = text(field(b.service, "name"));
        let port = field(b.service, "port");
        if port == () { port = #{}; }
        if backend_resolves(objs, ns, svc_name, port) { continue; }
        out.push(render_lint::finding(
            "dangling-ingress",
            "error",
            obj,
            b.path,
            `Ingress "${name}" : le backend Service "${svc_name}" est absent du namespace "${ns}" ou n'a pas le port demandé ; créer le Service ou corriger ${b.path}`
        ));
    }
    out
}
