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
    import "render_lint_network" as network;
    import "render_lint_workload" as workload;
    let cfg = settings(opts);
    let raw = [];
    for obj in objs {
        if enabled(cfg, "dangling-horizontalpodautoscaler") {
            raw = run_check(raw, cfg, "dangling-horizontalpodautoscaler", workload::dangling_horizontalpodautoscaler(obj, objs, ctx, param(cfg, "dangling-horizontalpodautoscaler")));
        }
        if enabled(cfg, "dangling-ingress") {
            raw = run_check(raw, cfg, "dangling-ingress", network::dangling_ingress(obj, objs, ctx, param(cfg, "dangling-ingress")));
        }
        if enabled(cfg, "dangling-networkpolicy") {
            raw = run_check(raw, cfg, "dangling-networkpolicy", network::dangling_networkpolicy(obj, objs, ctx, param(cfg, "dangling-networkpolicy")));
        }
        if enabled(cfg, "dangling-networkpolicypeer-podselector") {
            raw = run_check(raw, cfg, "dangling-networkpolicypeer-podselector", network::dangling_networkpolicypeer_podselector(obj, objs, ctx, param(cfg, "dangling-networkpolicypeer-podselector")));
        }
        if enabled(cfg, "dangling-service") {
            raw = run_check(raw, cfg, "dangling-service", network::dangling_service(obj, objs, ctx, param(cfg, "dangling-service")));
        }
        if enabled(cfg, "default-service-account") {
            raw = run_check(raw, cfg, "default-service-account", pod::default_service_account(obj, objs, ctx, param(cfg, "default-service-account")));
        }
        if enabled(cfg, "deprecated-service-account-field") {
            raw = run_check(raw, cfg, "deprecated-service-account-field", workload::deprecated_service_account_field(obj, objs, ctx, param(cfg, "deprecated-service-account-field")));
        }
        if enabled(cfg, "dnsconfig-options") {
            raw = run_check(raw, cfg, "dnsconfig-options", pod::dnsconfig_options(obj, objs, ctx, param(cfg, "dnsconfig-options")));
        }
        if enabled(cfg, "docker-sock") {
            raw = run_check(raw, cfg, "docker-sock", pod::docker_sock(obj, objs, ctx, param(cfg, "docker-sock")));
        }
        if enabled(cfg, "drop-net-raw-capability") {
            raw = run_check(raw, cfg, "drop-net-raw-capability", pod::drop_net_raw_capability(obj, objs, ctx, param(cfg, "drop-net-raw-capability")));
        }
        if enabled(cfg, "duplicate-env-var") {
            raw = run_check(raw, cfg, "duplicate-env-var", pod::duplicate_env_var(obj, objs, ctx, param(cfg, "duplicate-env-var")));
        }
        if enabled(cfg, "env-value-from") {
            raw = run_check(raw, cfg, "env-value-from", pod::env_value_from(obj, objs, ctx, param(cfg, "env-value-from")));
        }
        if enabled(cfg, "env-var-secret") {
            raw = run_check(raw, cfg, "env-var-secret", pod::env_var_secret(obj, objs, ctx, param(cfg, "env-var-secret")));
        }
        if enabled(cfg, "exposed-services") {
            raw = run_check(raw, cfg, "exposed-services", network::exposed_services(obj, objs, ctx, param(cfg, "exposed-services")));
        }
        if enabled(cfg, "host-ipc") {
            raw = run_check(raw, cfg, "host-ipc", pod::host_ipc(obj, objs, ctx, param(cfg, "host-ipc")));
        }
        if enabled(cfg, "host-network") {
            raw = run_check(raw, cfg, "host-network", pod::host_network(obj, objs, ctx, param(cfg, "host-network")));
        }
        if enabled(cfg, "host-pid") {
            raw = run_check(raw, cfg, "host-pid", pod::host_pid(obj, objs, ctx, param(cfg, "host-pid")));
        }
        if enabled(cfg, "hpa-minimum-three-replicas") {
            raw = run_check(raw, cfg, "hpa-minimum-three-replicas", workload::hpa_minimum_three_replicas(obj, objs, ctx, param(cfg, "hpa-minimum-three-replicas")));
        }
        if enabled(cfg, "invalid-target-ports") {
            raw = run_check(raw, cfg, "invalid-target-ports", network::invalid_target_ports(obj, objs, ctx, param(cfg, "invalid-target-ports")));
        }
        if enabled(cfg, "job-ttl-seconds-after-finished") {
            raw = run_check(raw, cfg, "job-ttl-seconds-after-finished", workload::job_ttl_seconds_after_finished(obj, objs, ctx, param(cfg, "job-ttl-seconds-after-finished")));
        }
        if enabled(cfg, "latest-tag") {
            raw = run_check(raw, cfg, "latest-tag", pod::latest_tag(obj, objs, ctx, param(cfg, "latest-tag")));
        }
        if enabled(cfg, "liveness-port") {
            raw = run_check(raw, cfg, "liveness-port", pod::liveness_port(obj, objs, ctx, param(cfg, "liveness-port")));
        }
        if enabled(cfg, "minimum-three-replicas") {
            raw = run_check(raw, cfg, "minimum-three-replicas", workload::minimum_three_replicas(obj, objs, ctx, param(cfg, "minimum-three-replicas")));
        }
        if enabled(cfg, "mismatching-selector") {
            raw = run_check(raw, cfg, "mismatching-selector", workload::mismatching_selector(obj, objs, ctx, param(cfg, "mismatching-selector")));
        }
        if enabled(cfg, "no-anti-affinity") {
            raw = run_check(raw, cfg, "no-anti-affinity", workload::no_anti_affinity(obj, objs, ctx, param(cfg, "no-anti-affinity")));
        }
        if enabled(cfg, "no-extensions-v1beta") {
            raw = run_check(raw, cfg, "no-extensions-v1beta", workload::no_extensions_v1beta(obj, objs, ctx, param(cfg, "no-extensions-v1beta")));
        }
        if enabled(cfg, "no-liveness-probe") {
            raw = run_check(raw, cfg, "no-liveness-probe", pod::no_liveness_probe(obj, objs, ctx, param(cfg, "no-liveness-probe")));
        }
        if enabled(cfg, "no-node-affinity") {
            raw = run_check(raw, cfg, "no-node-affinity", workload::no_node_affinity(obj, objs, ctx, param(cfg, "no-node-affinity")));
        }
        if enabled(cfg, "no-read-only-root-fs") {
            raw = run_check(raw, cfg, "no-read-only-root-fs", pod::no_read_only_root_fs(obj, objs, ctx, param(cfg, "no-read-only-root-fs")));
        }
        if enabled(cfg, "no-readiness-probe") {
            raw = run_check(raw, cfg, "no-readiness-probe", pod::no_readiness_probe(obj, objs, ctx, param(cfg, "no-readiness-probe")));
        }
        if enabled(cfg, "no-rolling-update-strategy") {
            raw = run_check(raw, cfg, "no-rolling-update-strategy", workload::no_rolling_update_strategy(obj, objs, ctx, param(cfg, "no-rolling-update-strategy")));
        }
        if enabled(cfg, "non-isolated-pod") {
            raw = run_check(raw, cfg, "non-isolated-pod", network::non_isolated_pod(obj, objs, ctx, param(cfg, "non-isolated-pod")));
        }
        if enabled(cfg, "pdb-max-unavailable") {
            raw = run_check(raw, cfg, "pdb-max-unavailable", workload::pdb_max_unavailable(obj, objs, ctx, param(cfg, "pdb-max-unavailable")));
        }
        if enabled(cfg, "pdb-min-available") {
            raw = run_check(raw, cfg, "pdb-min-available", workload::pdb_min_available(obj, objs, ctx, param(cfg, "pdb-min-available")));
        }
        if enabled(cfg, "pdb-unhealthy-pod-eviction-policy") {
            raw = run_check(raw, cfg, "pdb-unhealthy-pod-eviction-policy", workload::pdb_unhealthy_pod_eviction_policy(obj, objs, ctx, param(cfg, "pdb-unhealthy-pod-eviction-policy")));
        }
        if enabled(cfg, "priority-class-name") {
            raw = run_check(raw, cfg, "priority-class-name", pod::priority_class_name(obj, objs, ctx, param(cfg, "priority-class-name")));
        }
        if enabled(cfg, "privilege-escalation-container") {
            raw = run_check(raw, cfg, "privilege-escalation-container", pod::privilege_escalation_container(obj, objs, ctx, param(cfg, "privilege-escalation-container")));
        }
        if enabled(cfg, "privileged-container") {
            raw = run_check(raw, cfg, "privileged-container", pod::privileged_container(obj, objs, ctx, param(cfg, "privileged-container")));
        }
        if enabled(cfg, "privileged-ports") {
            raw = run_check(raw, cfg, "privileged-ports", pod::privileged_ports(obj, objs, ctx, param(cfg, "privileged-ports")));
        }
        if enabled(cfg, "read-secret-from-env-var") {
            raw = run_check(raw, cfg, "read-secret-from-env-var", pod::read_secret_from_env_var(obj, objs, ctx, param(cfg, "read-secret-from-env-var")));
        }
        if enabled(cfg, "readiness-port") {
            raw = run_check(raw, cfg, "readiness-port", pod::readiness_port(obj, objs, ctx, param(cfg, "readiness-port")));
        }
        if enabled(cfg, "required-annotation-email") {
            raw = run_check(raw, cfg, "required-annotation-email", workload::required_annotation_email(obj, objs, ctx, param(cfg, "required-annotation-email")));
        }
        if enabled(cfg, "required-label-owner") {
            raw = run_check(raw, cfg, "required-label-owner", workload::required_label_owner(obj, objs, ctx, param(cfg, "required-label-owner")));
        }
        if enabled(cfg, "restart-policy") {
            raw = run_check(raw, cfg, "restart-policy", pod::restart_policy(obj, objs, ctx, param(cfg, "restart-policy")));
        }
        if enabled(cfg, "run-as-non-root") {
            raw = run_check(raw, cfg, "run-as-non-root", pod::run_as_non_root(obj, objs, ctx, param(cfg, "run-as-non-root")));
        }
        if enabled(cfg, "sensitive-host-mounts") {
            raw = run_check(raw, cfg, "sensitive-host-mounts", pod::sensitive_host_mounts(obj, objs, ctx, param(cfg, "sensitive-host-mounts")));
        }
        if enabled(cfg, "ssh-port") {
            raw = run_check(raw, cfg, "ssh-port", pod::ssh_port(obj, objs, ctx, param(cfg, "ssh-port")));
        }
        if enabled(cfg, "startup-port") {
            raw = run_check(raw, cfg, "startup-port", pod::startup_port(obj, objs, ctx, param(cfg, "startup-port")));
        }
        if enabled(cfg, "unsafe-proc-mount") {
            raw = run_check(raw, cfg, "unsafe-proc-mount", pod::unsafe_proc_mount(obj, objs, ctx, param(cfg, "unsafe-proc-mount")));
        }
        if enabled(cfg, "unsafe-sysctls") {
            raw = run_check(raw, cfg, "unsafe-sysctls", pod::unsafe_sysctls(obj, objs, ctx, param(cfg, "unsafe-sysctls")));
        }
        if enabled(cfg, "unset-cpu-requirements") {
            raw = run_check(raw, cfg, "unset-cpu-requirements", pod::unset_cpu_requirements(obj, objs, ctx, param(cfg, "unset-cpu-requirements")));
        }
        if enabled(cfg, "unset-memory-requirements") {
            raw = run_check(raw, cfg, "unset-memory-requirements", pod::unset_memory_requirements(obj, objs, ctx, param(cfg, "unset-memory-requirements")));
        }
        if enabled(cfg, "use-namespace") {
            raw = run_check(raw, cfg, "use-namespace", workload::use_namespace(obj, objs, ctx, param(cfg, "use-namespace")));
        }
        if enabled(cfg, "writable-host-mount") {
            raw = run_check(raw, cfg, "writable-host-mount", pod::writable_host_mount(obj, objs, ctx, param(cfg, "writable-host-mount")));
        }
    }
    apply_suppressions(raw, objs, #{ ignore_prefix: cfg.ignore_prefix, require_reason: cfg.require_reason })
}

// Noms des vérifications portées, triés. Doit correspondre aux entrées `ported` du catalogue
// (fixtures/render_lint/catalogue.yaml) ; le test de cohérence le vérifie.
fn checks() {
    ["dangling-horizontalpodautoscaler", "dangling-ingress", "dangling-networkpolicy", "dangling-networkpolicypeer-podselector", "dangling-service", "default-service-account", "deprecated-service-account-field", "dnsconfig-options", "docker-sock", "drop-net-raw-capability", "duplicate-env-var", "env-value-from", "env-var-secret", "exposed-services", "host-ipc", "host-network", "host-pid", "hpa-minimum-three-replicas", "invalid-target-ports", "job-ttl-seconds-after-finished", "latest-tag", "liveness-port", "minimum-three-replicas", "mismatching-selector", "no-anti-affinity", "no-extensions-v1beta", "no-liveness-probe", "no-node-affinity", "no-read-only-root-fs", "no-readiness-probe", "no-rolling-update-strategy", "non-isolated-pod", "pdb-max-unavailable", "pdb-min-available", "pdb-unhealthy-pod-eviction-policy", "priority-class-name", "privilege-escalation-container", "privileged-container", "privileged-ports", "read-secret-from-env-var", "readiness-port", "required-annotation-email", "required-label-owner", "restart-policy", "run-as-non-root", "sensitive-host-mounts", "ssh-port", "startup-port", "unsafe-proc-mount", "unsafe-sysctls", "unset-cpu-requirements", "unset-memory-requirements", "use-namespace", "writable-host-mount"]
}
