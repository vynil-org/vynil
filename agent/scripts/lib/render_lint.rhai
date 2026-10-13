// Attribution : les vérifications de ce module reprennent la logique de kube-linter
// (https://github.com/stackrox/kube-linter, licence Apache 2.0) : noms, paramètres, valeurs par
// défaut et sémantique mesurée sur v0.8.3. Voir /NOTICE et /third_party/kube-linter/LICENSE.
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

// Groupe d'API d'une apiVersion : partie avant `/` (`apps` pour `apps/v1`), ou `""` (groupe du
// noyau) pour `v1`. Une apiVersion sans `/` qui n'est pas `v1` (`v1beta1`) n'a pas de groupe : `()`.
// Toutes les versions d'un même groupe sont équivalentes, comme dans kube-linter.
fn api_group_of(api) {
    let parts = api.split("/");
    if parts.len() == 2 { parts[0] } else if api == "v1" { "" } else { () }
}

// Vrai quand l'objet a l'un des genres `kinds` dans le groupe d'API `group` (`""` pour le noyau).
// Seule identification d'un objet par son genre : un kind seul ne suffit jamais.
fn kind_of_group(obj, kinds, group) {
    let kind = text(field(obj, "kind"));
    (kind in kinds) && (api_group_of(text(field(obj, "apiVersion"))) == group)
}

// Gabarits de pod d'un objet : `#{ pod, path }`. `path` est `spec.template`, `spec.jobTemplate.spec.template`
// ou "" pour un Pod. Tableau vide pour un objet sans pod.
fn pod_templates(obj) {
    // Sortie rapide : la plupart des objets d'un rendu n'embarquent pas de pod, et chaque
    // vérification de pod appelle cette fonction pour chaque objet.
    let kind = text(field(obj, "kind"));
    if !(kind in ["Pod", "Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job", "CronJob", "ReplicationController"]) {
        return [];
    }
    let spec = field(obj, "spec");
    let path = "";
    let template = ();
    if kind_of_group(obj, ["Pod"], "") {
        if type_of(spec) != "map" { return []; }
        return [#{ pod: obj, path: "" }];
    } else if kind_of_group(obj, ["Deployment", "StatefulSet", "DaemonSet", "ReplicaSet"], "apps")
        || kind_of_group(obj, ["Job"], "batch")
        || kind_of_group(obj, ["ReplicationController"], "") {
        path = "spec.template";
        template = field(spec, "template");
    } else if kind_of_group(obj, ["CronJob"], "batch") {
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

// Marque les constats dont l'objet porte l'annotation de suppression. Sans `ignore_prefix`,
// rien n'est supprimé. Avec `require_reason`, une annotation vide ne supprime rien et ajoute un
// constat `suppression-without-reason` (une fois par objet et règle). Sans `require_reason`, une
// annotation vide supprime avec la raison explicite `(aucune raison donnée)` : le moteur refuse
// une suppression sans raison, et une exception muette doit rester lisible dans le rapport.
fn apply_suppressions(findings, objs, opts) {
    let prefix = field(opts, "ignore_prefix");
    if type_of(prefix) != "string" { return findings; }
    let require = field(opts, "require_reason") == true;
    // Index des objets par identité, construit une fois : un parcours par constat serait O(F x N).
    let by_identity = multimap();
    for o in objs { by_identity.put(identity(object_ref(o)), o); }
    let out = [];
    let extra = [];
    let seen = #{};
    for f in findings {
        let obj = by_identity.first(identity(f.object));
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
        g.reason = if reason != "" { reason } else { "(aucune raison donnée)" };
        g.origin = "annotation:" + prefix + f.rule;
        out.push(g);
    }
    out += extra;
    out
}

// Types attendus des paramètres de chaque vérification (noms de paramètres de kube-linter) :
// `list` tableau de chaînes, `string`, `int`, `number` (entier ou flottant), ou une énumération
// `a|b|c`. Une vérification absente de la table n'a aucun paramètre.
private fn param_schema() {
    #{
        "access-to-create-pods": #{ "resources": "list", "verbs": "list" },
        "access-to-secrets": #{ "resources": "list", "verbs": "list" },
        "default-service-account": #{ "serviceAccount": "string" },
        "dnsconfig-options": #{ "Key": "string", "Value": "string" },
        "docker-sock": #{ "dirs": "list" },
        "drop-net-raw-capability": #{ "forbiddenCapabilities": "list" },
        "env-value-from": #{ "IgnoredConfigMaps": "list", "IgnoredSecrets": "list" },
        "env-var-secret": #{ "name": "string", "value": "string" },
        "exposed-services": #{ "forbiddenServiceTypes": "list" },
        "hpa-minimum-three-replicas": #{ "minReplicas": "int" },
        "latest-tag": #{ "BlockList": "list" },
        "minimum-three-replicas": #{ "minReplicas": "int" },
        "no-anti-affinity": #{ "minReplicas": "int" },
        "no-extensions-v1beta": #{ "group": "string", "version": "string" },
        "no-rolling-update-strategy": #{ "strategyTypeRegex": "string" },
        "priority-class-name": #{ "acceptedPriorityClassNames": "list" },
        "required-annotation-email": #{ "key": "string", "value": "string" },
        "required-label-owner": #{ "key": "string" },
        "sensitive-host-mounts": #{ "dirs": "list" },
        "ssh-port": #{ "port": "number", "protocol": "string" },
        "unsafe-sysctls": #{ "unsafeSysCtls": "list" },
        "unset-cpu-requirements": #{ "requirementsType": "request|limit|any", "lowerBoundMillis": "number", "upperBoundMillis": "number" },
        "unset-memory-requirements": #{ "requirementsType": "request|limit|any", "lowerBoundMB": "number", "upperBoundMB": "number" }
    }
}

// Vérifie le type de la valeur d'un paramètre ; lève une erreur qui nomme la vérification et le
// paramètre. Une valeur de paramètre invalide ne doit jamais désactiver la vérification sans bruit.
private fn check_param(check, key, expected, value) {
    let ok = if expected == "list" {
        let all_strings = type_of(value) == "array";
        if all_strings { for v in value { if type_of(v) != "string" { all_strings = false; } } }
        all_strings
    } else if expected == "string" {
        type_of(value) == "string"
    } else if expected == "int" {
        type_of(value) == "i64"
    } else if expected == "number" {
        type_of(value) == "i64" || type_of(value) == "f64"
    } else {
        type_of(value) == "string" && value in expected.split("|")
    };
    if !ok {
        let wanted = if expected == "list" { "un tableau de chaînes" }
            else if expected == "string" { "une chaîne" }
            else if expected == "int" { "un entier" }
            else if expected == "number" { "un nombre" }
            else { `l'une de ${expected.split("|")}` };
        throw `render_lint: params["${check}"].${key} : ${wanted} attendu, reçu ${type_of(value)} (${value})`;
    }
}

// Liste de noms de vérifications d'une option : tableau de noms connus, sinon erreur.
private fn names_option(opts, key, known) {
    let v = field(opts, key);
    if v == () { return (); }
    if type_of(v) != "array" {
        throw `render_lint: option "${key}" : un tableau de noms de vérifications est attendu, reçu ${type_of(v)}`;
    }
    for n in v {
        if type_of(n) != "string" || !(n in known) {
            throw `render_lint: option "${key}" : vérification inconnue ${n} (voir render_lint::checks())`;
        }
    }
    v
}

// Options normalisées et validées : chaque clé présente, valeurs par défaut appliquées. Toute
// clé ou valeur qui n'a pas de sens est une erreur du script (donc une ligne `lint:execution`) :
// une option qui ne s'applique pas parce qu'elle est mal écrite ne doit jamais passer en silence.
private fn settings(opts) {
    if opts == () { opts = #{}; }
    if type_of(opts) != "map" {
        throw `render_lint: opts doit être une carte, reçu ${type_of(opts)}`;
    }
    let known_options = ["exclude", "only", "severity", "params", "ignore_prefix", "require_reason", "include_optional"];
    for k in opts.keys() {
        if !(k in known_options) {
            throw `render_lint: option inconnue "${k}" (options : ${known_options})`;
        }
    }
    let known = checks();
    let exclude = names_option(opts, "exclude", known);
    if exclude == () { exclude = []; }
    let only = names_option(opts, "only", known);
    let severity = field(opts, "severity");
    if severity == () { severity = #{}; }
    if type_of(severity) != "map" {
        throw `render_lint: option "severity" : une carte nom -> "error" ou "warn" est attendue, reçu ${type_of(severity)}`;
    }
    for name in severity.keys() {
        if !(name in known) {
            throw `render_lint: option "severity" : vérification inconnue "${name}" (voir render_lint::checks())`;
        }
        if severity[name] != "error" && severity[name] != "warn" {
            throw `render_lint: option "severity" : "${name}" doit valoir "error" ou "warn", reçu ${severity[name]}`;
        }
    }
    let params = field(opts, "params");
    if params == () { params = #{}; }
    if type_of(params) != "map" {
        throw `render_lint: option "params" : une carte nom -> paramètres est attendue, reçu ${type_of(params)}`;
    }
    let schema = param_schema();
    for name in params.keys() {
        if !(name in known) {
            throw `render_lint: option "params" : vérification inconnue "${name}" (voir render_lint::checks())`;
        }
        let given = params[name];
        if type_of(given) != "map" {
            throw `render_lint: params["${name}"] : une carte de paramètres est attendue, reçu ${type_of(given)}`;
        }
        let accepted = if name in schema { schema[name] } else { #{} };
        for key in given.keys() {
            if !(key in accepted) {
                throw `render_lint: params["${name}"] : paramètre inconnu "${key}" (paramètres : ${accepted.keys()})`;
            }
            check_param(name, key, accepted[key], given[key]);
        }
    }
    let prefix = field(opts, "ignore_prefix");
    if prefix != () && type_of(prefix) != "string" {
        throw `render_lint: option "ignore_prefix" : une chaîne est attendue, reçu ${type_of(prefix)}`;
    }
    for flag in ["require_reason", "include_optional"] {
        let v = field(opts, flag);
        if v != () && type_of(v) != "bool" {
            throw `render_lint: option "${flag}" : un booléen est attendu, reçu ${type_of(v)}`;
        }
    }
    #{
        exclude: exclude,
        only: only,
        severity: severity,
        params: params,
        ignore_prefix: prefix,
        require_reason: field(opts, "require_reason") == true,
        include_optional: field(opts, "include_optional") == true
    }
}

// Carte nom -> booléen des vérifications exécutées. Avec `only`, exactement ces vérifications
// (une vérification facultative nommée est donc exécutée) ; sinon le jeu actif par défaut de
// kube-linter, ou toutes avec `include_optional`. `exclude` retire dans tous les cas.
private fn active_set(cfg) {
    let defaults = default_checks();
    let on = #{};
    for name in checks() {
        let selected = if type_of(cfg.only) == "array" { name in cfg.only }
            else if cfg.include_optional { true }
            else { name in defaults };
        on[name] = selected && !(name in cfg.exclude);
    }
    on
}

// Carte nom -> paramètres de chaque vérification (la carte donnée, ou une carte vide), construite
// une fois : `all` la lit par indexation à chaque objet et chaque vérification.
private fn params_by_check(cfg) {
    let out = #{};
    for name in checks() {
        let p = field(cfg.params, name);
        out[name] = if type_of(p) == "map" { p } else { #{} };
    }
    out
}

// Constats d'une vérification, avec la sévérité configurée appliquée.
private fn run_check(cfg, name, found) {
    let sev = field(cfg.severity, name);
    if type_of(sev) != "string" { return found; }
    let out = [];
    for f in found {
        let g = f;
        g.severity = sev;
        out.push(g);
    }
    out
}

// Index de la liste d'objets, construit une fois : poignées partagées (type `Multimap`) que les
// vérifications qui croisent plusieurs objets interrogent au lieu de reparcourir la liste. Rhai
// passe tout par valeur : donner la liste à chacune des soixante vérifications pour chaque objet
// la copiait 60 x N fois. Une distribution qui appelle une vérification directement construit
// l'index avec cette fonction et le passe en deuxième argument.
fn index(objs) {
    import "render_lint_pod" as pod;
    import "render_lint_network" as network;
    import "render_lint_rbac" as rbac;
    import "render_lint_workload" as workload;
    let idx = #{};
    idx += pod::index(objs);
    idx += network::index(objs);
    idx += rbac::index(objs);
    idx += workload::index(objs);
    idx
}

// Toutes les vérifications activées sur tous les objets, suppressions appliquées. Ordre : objets
// dans l'ordre, puis vérifications dans l'ordre alphabétique de leur nom. Sans option, le jeu actif
// par défaut de kube-linter (`default_checks()`), pas le catalogue entier.
//
// `ctx` est réservé : aucune vérification portée ne le lit, et le transmettre aux soixante
// vérifications le copierait 60 x N fois ; elles reçoivent `()`.
fn all(objs, ctx) { all(objs, ctx, #{}) }

fn all(objs, ctx, opts) {
    import "render_lint_pod" as pod;
    import "render_lint_network" as network;
    import "render_lint_rbac" as rbac;
    import "render_lint_workload" as workload;
    let cfg = settings(opts);
    let on = active_set(cfg);
    let pm = params_by_check(cfg);
    let idx = index(objs);
    let raw = [];
    for obj in objs {
        if on["access-to-create-pods"] {
            let found = rbac::access_to_create_pods(obj, idx, (), pm["access-to-create-pods"]);
            if found.len() > 0 { raw += run_check(cfg, "access-to-create-pods", found); }
        }
        if on["access-to-secrets"] {
            let found = rbac::access_to_secrets(obj, idx, (), pm["access-to-secrets"]);
            if found.len() > 0 { raw += run_check(cfg, "access-to-secrets", found); }
        }
        if on["cluster-admin-role-binding"] {
            let found = rbac::cluster_admin_role_binding(obj, idx, (), pm["cluster-admin-role-binding"]);
            if found.len() > 0 { raw += run_check(cfg, "cluster-admin-role-binding", found); }
        }
        if on["dangling-horizontalpodautoscaler"] {
            let found = workload::dangling_horizontalpodautoscaler(obj, idx, (), pm["dangling-horizontalpodautoscaler"]);
            if found.len() > 0 { raw += run_check(cfg, "dangling-horizontalpodautoscaler", found); }
        }
        if on["dangling-ingress"] {
            let found = network::dangling_ingress(obj, idx, (), pm["dangling-ingress"]);
            if found.len() > 0 { raw += run_check(cfg, "dangling-ingress", found); }
        }
        if on["dangling-networkpolicy"] {
            let found = network::dangling_networkpolicy(obj, idx, (), pm["dangling-networkpolicy"]);
            if found.len() > 0 { raw += run_check(cfg, "dangling-networkpolicy", found); }
        }
        if on["dangling-networkpolicypeer-podselector"] {
            let found = network::dangling_networkpolicypeer_podselector(obj, idx, (), pm["dangling-networkpolicypeer-podselector"]);
            if found.len() > 0 { raw += run_check(cfg, "dangling-networkpolicypeer-podselector", found); }
        }
        if on["dangling-service"] {
            let found = network::dangling_service(obj, idx, (), pm["dangling-service"]);
            if found.len() > 0 { raw += run_check(cfg, "dangling-service", found); }
        }
        if on["dangling-servicemonitor"] {
            let found = network::dangling_servicemonitor(obj, idx, (), pm["dangling-servicemonitor"]);
            if found.len() > 0 { raw += run_check(cfg, "dangling-servicemonitor", found); }
        }
        if on["default-service-account"] {
            let found = pod::default_service_account(obj, idx, (), pm["default-service-account"]);
            if found.len() > 0 { raw += run_check(cfg, "default-service-account", found); }
        }
        if on["deprecated-service-account-field"] {
            let found = workload::deprecated_service_account_field(obj, idx, (), pm["deprecated-service-account-field"]);
            if found.len() > 0 { raw += run_check(cfg, "deprecated-service-account-field", found); }
        }
        if on["dnsconfig-options"] {
            let found = pod::dnsconfig_options(obj, idx, (), pm["dnsconfig-options"]);
            if found.len() > 0 { raw += run_check(cfg, "dnsconfig-options", found); }
        }
        if on["docker-sock"] {
            let found = pod::docker_sock(obj, idx, (), pm["docker-sock"]);
            if found.len() > 0 { raw += run_check(cfg, "docker-sock", found); }
        }
        if on["drop-net-raw-capability"] {
            let found = pod::drop_net_raw_capability(obj, idx, (), pm["drop-net-raw-capability"]);
            if found.len() > 0 { raw += run_check(cfg, "drop-net-raw-capability", found); }
        }
        if on["duplicate-env-var"] {
            let found = pod::duplicate_env_var(obj, idx, (), pm["duplicate-env-var"]);
            if found.len() > 0 { raw += run_check(cfg, "duplicate-env-var", found); }
        }
        if on["env-value-from"] {
            let found = pod::env_value_from(obj, idx, (), pm["env-value-from"]);
            if found.len() > 0 { raw += run_check(cfg, "env-value-from", found); }
        }
        if on["env-var-secret"] {
            let found = pod::env_var_secret(obj, idx, (), pm["env-var-secret"]);
            if found.len() > 0 { raw += run_check(cfg, "env-var-secret", found); }
        }
        if on["exposed-services"] {
            let found = network::exposed_services(obj, idx, (), pm["exposed-services"]);
            if found.len() > 0 { raw += run_check(cfg, "exposed-services", found); }
        }
        if on["host-ipc"] {
            let found = pod::host_ipc(obj, idx, (), pm["host-ipc"]);
            if found.len() > 0 { raw += run_check(cfg, "host-ipc", found); }
        }
        if on["host-network"] {
            let found = pod::host_network(obj, idx, (), pm["host-network"]);
            if found.len() > 0 { raw += run_check(cfg, "host-network", found); }
        }
        if on["host-pid"] {
            let found = pod::host_pid(obj, idx, (), pm["host-pid"]);
            if found.len() > 0 { raw += run_check(cfg, "host-pid", found); }
        }
        if on["hpa-minimum-three-replicas"] {
            let found = workload::hpa_minimum_three_replicas(obj, idx, (), pm["hpa-minimum-three-replicas"]);
            if found.len() > 0 { raw += run_check(cfg, "hpa-minimum-three-replicas", found); }
        }
        if on["invalid-target-ports"] {
            let found = network::invalid_target_ports(obj, idx, (), pm["invalid-target-ports"]);
            if found.len() > 0 { raw += run_check(cfg, "invalid-target-ports", found); }
        }
        if on["job-ttl-seconds-after-finished"] {
            let found = workload::job_ttl_seconds_after_finished(obj, idx, (), pm["job-ttl-seconds-after-finished"]);
            if found.len() > 0 { raw += run_check(cfg, "job-ttl-seconds-after-finished", found); }
        }
        if on["latest-tag"] {
            let found = pod::latest_tag(obj, idx, (), pm["latest-tag"]);
            if found.len() > 0 { raw += run_check(cfg, "latest-tag", found); }
        }
        if on["liveness-port"] {
            let found = pod::liveness_port(obj, idx, (), pm["liveness-port"]);
            if found.len() > 0 { raw += run_check(cfg, "liveness-port", found); }
        }
        if on["minimum-three-replicas"] {
            let found = workload::minimum_three_replicas(obj, idx, (), pm["minimum-three-replicas"]);
            if found.len() > 0 { raw += run_check(cfg, "minimum-three-replicas", found); }
        }
        if on["mismatching-selector"] {
            let found = workload::mismatching_selector(obj, idx, (), pm["mismatching-selector"]);
            if found.len() > 0 { raw += run_check(cfg, "mismatching-selector", found); }
        }
        if on["no-anti-affinity"] {
            let found = workload::no_anti_affinity(obj, idx, (), pm["no-anti-affinity"]);
            if found.len() > 0 { raw += run_check(cfg, "no-anti-affinity", found); }
        }
        if on["no-extensions-v1beta"] {
            let found = workload::no_extensions_v1beta(obj, idx, (), pm["no-extensions-v1beta"]);
            if found.len() > 0 { raw += run_check(cfg, "no-extensions-v1beta", found); }
        }
        if on["no-liveness-probe"] {
            let found = pod::no_liveness_probe(obj, idx, (), pm["no-liveness-probe"]);
            if found.len() > 0 { raw += run_check(cfg, "no-liveness-probe", found); }
        }
        if on["no-node-affinity"] {
            let found = workload::no_node_affinity(obj, idx, (), pm["no-node-affinity"]);
            if found.len() > 0 { raw += run_check(cfg, "no-node-affinity", found); }
        }
        if on["no-read-only-root-fs"] {
            let found = pod::no_read_only_root_fs(obj, idx, (), pm["no-read-only-root-fs"]);
            if found.len() > 0 { raw += run_check(cfg, "no-read-only-root-fs", found); }
        }
        if on["no-readiness-probe"] {
            let found = pod::no_readiness_probe(obj, idx, (), pm["no-readiness-probe"]);
            if found.len() > 0 { raw += run_check(cfg, "no-readiness-probe", found); }
        }
        if on["no-rolling-update-strategy"] {
            let found = workload::no_rolling_update_strategy(obj, idx, (), pm["no-rolling-update-strategy"]);
            if found.len() > 0 { raw += run_check(cfg, "no-rolling-update-strategy", found); }
        }
        if on["non-existent-service-account"] {
            let found = rbac::non_existent_service_account(obj, idx, (), pm["non-existent-service-account"]);
            if found.len() > 0 { raw += run_check(cfg, "non-existent-service-account", found); }
        }
        if on["non-isolated-pod"] {
            let found = network::non_isolated_pod(obj, idx, (), pm["non-isolated-pod"]);
            if found.len() > 0 { raw += run_check(cfg, "non-isolated-pod", found); }
        }
        if on["pdb-max-unavailable"] {
            let found = workload::pdb_max_unavailable(obj, idx, (), pm["pdb-max-unavailable"]);
            if found.len() > 0 { raw += run_check(cfg, "pdb-max-unavailable", found); }
        }
        if on["pdb-min-available"] {
            let found = workload::pdb_min_available(obj, idx, (), pm["pdb-min-available"]);
            if found.len() > 0 { raw += run_check(cfg, "pdb-min-available", found); }
        }
        if on["pdb-unhealthy-pod-eviction-policy"] {
            let found = workload::pdb_unhealthy_pod_eviction_policy(obj, idx, (), pm["pdb-unhealthy-pod-eviction-policy"]);
            if found.len() > 0 { raw += run_check(cfg, "pdb-unhealthy-pod-eviction-policy", found); }
        }
        if on["priority-class-name"] {
            let found = pod::priority_class_name(obj, idx, (), pm["priority-class-name"]);
            if found.len() > 0 { raw += run_check(cfg, "priority-class-name", found); }
        }
        if on["privilege-escalation-container"] {
            let found = pod::privilege_escalation_container(obj, idx, (), pm["privilege-escalation-container"]);
            if found.len() > 0 { raw += run_check(cfg, "privilege-escalation-container", found); }
        }
        if on["privileged-container"] {
            let found = pod::privileged_container(obj, idx, (), pm["privileged-container"]);
            if found.len() > 0 { raw += run_check(cfg, "privileged-container", found); }
        }
        if on["privileged-ports"] {
            let found = pod::privileged_ports(obj, idx, (), pm["privileged-ports"]);
            if found.len() > 0 { raw += run_check(cfg, "privileged-ports", found); }
        }
        if on["read-secret-from-env-var"] {
            let found = pod::read_secret_from_env_var(obj, idx, (), pm["read-secret-from-env-var"]);
            if found.len() > 0 { raw += run_check(cfg, "read-secret-from-env-var", found); }
        }
        if on["readiness-port"] {
            let found = pod::readiness_port(obj, idx, (), pm["readiness-port"]);
            if found.len() > 0 { raw += run_check(cfg, "readiness-port", found); }
        }
        if on["required-annotation-email"] {
            let found = workload::required_annotation_email(obj, idx, (), pm["required-annotation-email"]);
            if found.len() > 0 { raw += run_check(cfg, "required-annotation-email", found); }
        }
        if on["required-label-owner"] {
            let found = workload::required_label_owner(obj, idx, (), pm["required-label-owner"]);
            if found.len() > 0 { raw += run_check(cfg, "required-label-owner", found); }
        }
        if on["restart-policy"] {
            let found = pod::restart_policy(obj, idx, (), pm["restart-policy"]);
            if found.len() > 0 { raw += run_check(cfg, "restart-policy", found); }
        }
        if on["run-as-non-root"] {
            let found = pod::run_as_non_root(obj, idx, (), pm["run-as-non-root"]);
            if found.len() > 0 { raw += run_check(cfg, "run-as-non-root", found); }
        }
        if on["sensitive-host-mounts"] {
            let found = pod::sensitive_host_mounts(obj, idx, (), pm["sensitive-host-mounts"]);
            if found.len() > 0 { raw += run_check(cfg, "sensitive-host-mounts", found); }
        }
        if on["ssh-port"] {
            let found = pod::ssh_port(obj, idx, (), pm["ssh-port"]);
            if found.len() > 0 { raw += run_check(cfg, "ssh-port", found); }
        }
        if on["startup-port"] {
            let found = pod::startup_port(obj, idx, (), pm["startup-port"]);
            if found.len() > 0 { raw += run_check(cfg, "startup-port", found); }
        }
        if on["unsafe-proc-mount"] {
            let found = pod::unsafe_proc_mount(obj, idx, (), pm["unsafe-proc-mount"]);
            if found.len() > 0 { raw += run_check(cfg, "unsafe-proc-mount", found); }
        }
        if on["unsafe-sysctls"] {
            let found = pod::unsafe_sysctls(obj, idx, (), pm["unsafe-sysctls"]);
            if found.len() > 0 { raw += run_check(cfg, "unsafe-sysctls", found); }
        }
        if on["unset-cpu-requirements"] {
            let found = pod::unset_cpu_requirements(obj, idx, (), pm["unset-cpu-requirements"]);
            if found.len() > 0 { raw += run_check(cfg, "unset-cpu-requirements", found); }
        }
        if on["unset-memory-requirements"] {
            let found = pod::unset_memory_requirements(obj, idx, (), pm["unset-memory-requirements"]);
            if found.len() > 0 { raw += run_check(cfg, "unset-memory-requirements", found); }
        }
        if on["use-namespace"] {
            let found = workload::use_namespace(obj, idx, (), pm["use-namespace"]);
            if found.len() > 0 { raw += run_check(cfg, "use-namespace", found); }
        }
        if on["wildcard-in-rules"] {
            let found = rbac::wildcard_in_rules(obj, idx, (), pm["wildcard-in-rules"]);
            if found.len() > 0 { raw += run_check(cfg, "wildcard-in-rules", found); }
        }
        if on["writable-host-mount"] {
            let found = pod::writable_host_mount(obj, idx, (), pm["writable-host-mount"]);
            if found.len() > 0 { raw += run_check(cfg, "writable-host-mount", found); }
        }
    }
    apply_suppressions(raw, objs, #{ ignore_prefix: cfg.ignore_prefix, require_reason: cfg.require_reason })
}

// Noms des vérifications portées, triés. Doit correspondre aux entrées `ported` du catalogue
// (fixtures/render_lint/catalogue.yaml) ; le test de cohérence le vérifie.
fn checks() {
    ["access-to-create-pods", "access-to-secrets", "cluster-admin-role-binding", "dangling-horizontalpodautoscaler", "dangling-ingress", "dangling-networkpolicy", "dangling-networkpolicypeer-podselector", "dangling-service", "dangling-servicemonitor", "default-service-account", "deprecated-service-account-field", "dnsconfig-options", "docker-sock", "drop-net-raw-capability", "duplicate-env-var", "env-value-from", "env-var-secret", "exposed-services", "host-ipc", "host-network", "host-pid", "hpa-minimum-three-replicas", "invalid-target-ports", "job-ttl-seconds-after-finished", "latest-tag", "liveness-port", "minimum-three-replicas", "mismatching-selector", "no-anti-affinity", "no-extensions-v1beta", "no-liveness-probe", "no-node-affinity", "no-read-only-root-fs", "no-readiness-probe", "no-rolling-update-strategy", "non-existent-service-account", "non-isolated-pod", "pdb-max-unavailable", "pdb-min-available", "pdb-unhealthy-pod-eviction-policy", "priority-class-name", "privilege-escalation-container", "privileged-container", "privileged-ports", "read-secret-from-env-var", "readiness-port", "required-annotation-email", "required-label-owner", "restart-policy", "run-as-non-root", "sensitive-host-mounts", "ssh-port", "startup-port", "unsafe-proc-mount", "unsafe-sysctls", "unset-cpu-requirements", "unset-memory-requirements", "use-namespace", "wildcard-in-rules", "writable-host-mount"]
}

// Noms des vérifications actives par défaut chez kube-linter (« Enabled by default: Yes »),
// triés. Doit correspondre aux entrées `ported` à `default: true` du catalogue.
fn default_checks() {
    ["dangling-service", "deprecated-service-account-field", "docker-sock", "drop-net-raw-capability", "duplicate-env-var", "env-var-secret", "host-ipc", "host-network", "host-pid", "invalid-target-ports", "job-ttl-seconds-after-finished", "latest-tag", "liveness-port", "mismatching-selector", "no-anti-affinity", "no-extensions-v1beta", "no-read-only-root-fs", "non-existent-service-account", "pdb-max-unavailable", "pdb-min-available", "pdb-unhealthy-pod-eviction-policy", "privilege-escalation-container", "privileged-container", "readiness-port", "run-as-non-root", "sensitive-host-mounts", "ssh-port", "startup-port", "unsafe-sysctls", "unset-cpu-requirements", "unset-memory-requirements"]
}
