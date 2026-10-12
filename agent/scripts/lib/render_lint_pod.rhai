// Thème « pod » des vérifications portées de kube-linter (spec : render_lint_pod.sdd).
// Chaque vérification est pure : elle ne lit que ses arguments et retourne un tableau de
// constats, vide quand rien n'est à signaler. `render_lint` est importé dans le corps de chaque
// fonction (et non en tête) : le noyau importe ce module, un import en tête ferait un cycle.

// Lit une clé d'une carte ; `()` si la valeur n'est pas une carte ou si la clé est absente.
private fn field(m, key) {
    if type_of(m) == "map" && key in m { m[key] } else { () }
}

// Chaîne, ou chaîne vide si la valeur n'en est pas une.
private fn text(v) {
    if type_of(v) == "string" { v } else { "" }
}

// Vrai quand `value` correspond à au moins un motif de `patterns` (expressions régulières, via
// `regex_is_match`, comme les listes de motifs de kube-linter : BlockList, dirs).
private fn matches_any(value, patterns) {
    for pattern in patterns {
        if type_of(pattern) == "string" && regex_is_match(pattern, value) { return true; }
    }
    false
}

// Liste du paramètre `key`, ou `fallback` si le paramètre est absent ou n'est pas un tableau.
private fn list_param(params, key, fallback) {
    let v = field(params, key);
    if type_of(v) == "array" { v } else { fallback }
}

// Chemin de la spec du pod d'une entrée de `pod_templates` : `spec` ou `spec.template.spec`.
private fn pod_spec_path(entry) {
    let base = text(field(entry, "path"));
    if base == "" { "spec" } else { base + ".spec" }
}

// Vrai quand la liste de capacités `drops` retire `cap` : `ALL`, ou une entrée qui contient `cap`
// (kube-linter compare par inclusion : `CAP_NET_RAW` vaut `NET_RAW`).
private fn drops_capability(drops, cap) {
    if type_of(drops) != "array" { return false; }
    for d in drops {
        if type_of(d) == "string" && (d == "ALL" || d.contains(cap)) { return true; }
    }
    false
}

// Vrai quand la liste `list` contient exactement `value`.
private fn contains_value(list, value) {
    type_of(list) == "array" && value in list
}

// Constats « pod partagé avec l'hôte » : le champ booléen `field_name` de la spec du pod vaut
// `true`. Un constat par objet.
private fn host_flag_findings(rule, field_name, what, obj) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        let spec = field(field(entry, "pod"), "spec");
        if field(spec, field_name) == true {
            findings.push(render_lint::finding(
                rule,
                "error",
                obj,
                pod_spec_path(entry) + "." + field_name,
                `${kind} "${name}" : ${what} est partagé avec l'hôte (${field_name}: true) ; définir ${field_name} à false ou l'omettre`
            ));
        }
    }
    findings
}

// Constats « montage d'un répertoire hôte sensible » : pour chaque conteneur (initialisation
// comprise) et chaque montage qui référence un volume `hostPath` dont le chemin correspond à
// `dirs`. Un constat par montage.
private fn host_mount_findings(rule, obj, dirs) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        let hostpaths = #{};
        let volumes = field(field(field(entry, "pod"), "spec"), "volumes");
        if type_of(volumes) == "array" {
            for v in volumes {
                let vname = field(v, "name");
                let path = field(field(v, "hostPath"), "path");
                if type_of(vname) == "string" && type_of(path) == "string" && matches_any(path, dirs) {
                    hostpaths[vname] = path;
                }
            }
        }
        for c in render_lint::containers(entry) {
            let mounts = field(c.container, "volumeMounts");
            if type_of(mounts) != "array" { continue; }
            for m in mounts {
                let vname = field(m, "name");
                if type_of(vname) == "string" && vname in hostpaths {
                    findings.push(render_lint::finding(
                        rule,
                        "error",
                        obj,
                        c.path + ".volumeMounts",
                        `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : le répertoire hôte "${hostpaths[vname]}" est monté (volume "${vname}") ; retirer ce volume hostPath et son montage`
                    ));
                }
            }
        }
    }
    findings
}

// Socket du démon Docker monté dans un conteneur (initialisation comprise).
fn docker_sock(obj, objs, ctx, params) {
    host_mount_findings("docker-sock", obj, list_param(params, "dirs", ["docker.sock$"]))
}

// Capacités NET_RAW : un conteneur qui ajoute une capacité interdite (ou `ALL`) produit un
// constat par entrée ; un conteneur dont `capabilities` est renseigné sans retirer la capacité
// interdite (ni `ALL`) produit un constat de retrait. Sans `securityContext.capabilities`, rien.
fn drop_net_raw_capability(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let forbidden = list_param(params, "forbiddenCapabilities", ["NET_RAW"]);
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            let caps = field(field(c.container, "securityContext"), "capabilities");
            if type_of(caps) != "map" { continue; }
            let cname = field(c.container, "name");
            for cap in forbidden {
                if type_of(cap) != "string" { continue; }
                let adds = field(caps, "add");
                if type_of(adds) == "array" {
                    for a in adds {
                        if type_of(a) == "string" && (a == "ALL" || a.contains(cap)) {
                            findings.push(render_lint::finding(
                                "drop-net-raw-capability",
                                "error",
                                obj,
                                c.path + ".securityContext.capabilities.add",
                                `conteneur "${cname}" de ${kind} "${name}" : capacité ajoutée "${a}" interdite (${cap}) ; retirer cette capacité de securityContext.capabilities.add`
                            ));
                        }
                    }
                }
                if !drops_capability(field(caps, "drop"), cap) {
                    findings.push(render_lint::finding(
                        "drop-net-raw-capability",
                        "error",
                        obj,
                        c.path + ".securityContext.capabilities.drop",
                        `conteneur "${cname}" de ${kind} "${name}" : capacités retirées ${field(caps, "drop")} sans "${cap}" ; ajouter "${cap}" ou "ALL" à securityContext.capabilities.drop`
                    ));
                }
            }
        }
    }
    findings
}

// Répertoire hôte sensible monté (`/`, `/etc`, `/proc`, ...), initialisation comprise. Les
// motifs `dirs` sont ceux de kube-linter, ancrés : `^/etc$` ne vise pas `/etc/foo`.
fn sensitive_host_mounts(obj, objs, ctx, params) {
    host_mount_findings("sensitive-host-mounts", obj, list_param(params, "dirs", [
        "^/$", "^/boot$", "^/dev$", "^/etc$", "^/lib$", "^/proc$", "^/sys$", "^/usr$"
    ]))
}

// Conteneur (initialisation comprise) dont `procMount` vaut `Unmasked`, sans tenir compte de la
// casse : kube-linter contourne alors le masquage /proc du runtime.
fn unsafe_proc_mount(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            let proc_mount = field(field(c.container, "securityContext"), "procMount");
            if type_of(proc_mount) == "string" && proc_mount.to_lower() == "unmasked" {
                findings.push(render_lint::finding(
                    "unsafe-proc-mount",
                    "error",
                    obj,
                    c.path + ".securityContext.procMount",
                    `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : procMount "${proc_mount}" expose /proc sans masquage ; définir procMount: Default`
                ));
            }
        }
    }
    findings
}

// Sysctl du pod dont le nom commence par un préfixe de `unsafeSysCtls` (préfixe brut, sensible
// à la casse : `net` ne vise pas `net.core`, mais `net.` si). Un constat par entrée.
fn unsafe_sysctls(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let prefixes = list_param(params, "unsafeSysCtls", [
        "kernel.msg", "kernel.sem", "kernel.shm", "fs.mqueue.", "net."
    ]);
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        let sysctls = field(field(field(field(entry, "pod"), "spec"), "securityContext"), "sysctls");
        if type_of(sysctls) != "array" { continue; }
        let i = 0;
        for sysctl in sysctls {
            let sname = field(sysctl, "name");
            if type_of(sname) == "string" {
                for prefix in prefixes {
                    if type_of(prefix) == "string" && sname.starts_with(prefix) {
                        findings.push(render_lint::finding(
                            "unsafe-sysctls",
                            "error",
                            obj,
                            `${pod_spec_path(entry)}.securityContext.sysctls[${i}]`,
                            `${kind} "${name}" : sysctl "${sname}" non sûr (préfixe "${prefix}") ; le retirer de securityContext.sysctls`
                        ));
                        break;
                    }
                }
            }
            i += 1;
        }
    }
    findings
}

// Namespace IPC de l'hôte partagé (`hostIPC: true`).
fn host_ipc(obj, objs, ctx, params) {
    host_flag_findings("host-ipc", "hostIPC", "l'espace IPC de l'hôte", obj)
}

// Namespace réseau de l'hôte partagé (`hostNetwork: true`).
fn host_network(obj, objs, ctx, params) {
    host_flag_findings("host-network", "hostNetwork", "l'espace réseau de l'hôte", obj)
}

// Namespace des processus de l'hôte partagé (`hostPID: true`).
fn host_pid(obj, objs, ctx, params) {
    host_flag_findings("host-pid", "hostPID", "l'espace des processus de l'hôte", obj)
}

// Conteneur qui permet l'escalade de privilèges : `allowPrivilegeEscalation: true`, ou
// `privileged: true`, ou la capacité `SYS_ADMIN` ajoutée (égalité exacte, comme kube-linter).
// Un constat par conteneur, pour la première condition qui s'applique.
fn privilege_escalation_container(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            let sc = field(c.container, "securityContext");
            let reason = "";
            if field(sc, "allowPrivilegeEscalation") == true {
                reason = "allowPrivilegeEscalation: true";
            } else if field(sc, "privileged") == true {
                reason = "privileged: true";
            } else if contains_value(field(field(sc, "capabilities"), "add"), "SYS_ADMIN") {
                reason = "capacité SYS_ADMIN ajoutée";
            }
            if reason != "" {
                findings.push(render_lint::finding(
                    "privilege-escalation-container",
                    "error",
                    obj,
                    c.path + ".securityContext",
                    `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : ${reason} ; permet l'escalade de privilèges, définir allowPrivilegeEscalation: false et privileged: false, et retirer SYS_ADMIN`
                ));
            }
        }
    }
    findings
}

// Conteneur (initialisation comprise) en mode privilégié : `privileged: true`.
fn privileged_container(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            if field(field(c.container, "securityContext"), "privileged") == true {
                findings.push(render_lint::finding(
                    "privileged-container",
                    "error",
                    obj,
                    c.path + ".securityContext.privileged",
                    `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : mode privilégié (privileged: true) ; passer privileged à false ou le retirer`
                ));
            }
        }
    }
    findings
}

// Conteneur (initialisation comprise) qui ne garantit pas l'exécution sans root. `runAsNonRoot`
// est celui du conteneur, à défaut celui du pod ; `runAsUser` suit la même règle. Un constat par
// conteneur : « non défini à true », sinon « runAsUser à 0 ».
fn run_as_non_root(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        let pod_sc = field(field(field(entry, "pod"), "spec"), "securityContext");
        for c in render_lint::containers(entry) {
            let cname = field(c.container, "name");
            let sc = field(c.container, "securityContext");
            let nonroot = field(sc, "runAsNonRoot");
            if nonroot == () { nonroot = field(pod_sc, "runAsNonRoot"); }
            if nonroot != true {
                findings.push(render_lint::finding(
                    "run-as-non-root",
                    "error",
                    obj,
                    c.path,
                    `conteneur "${cname}" de ${kind} "${name}" : runAsNonRoot n'est pas à true ; définir runAsNonRoot: true (sur le conteneur ou le pod) et un runAsUser non nul`
                ));
                continue;
            }
            let uid = field(sc, "runAsUser");
            if uid == () { uid = field(pod_sc, "runAsUser"); }
            if type_of(uid) == "i64" && uid == 0 {
                findings.push(render_lint::finding(
                    "run-as-non-root",
                    "error",
                    obj,
                    c.path,
                    `conteneur "${cname}" de ${kind} "${name}" : runAsNonRoot est à true mais runAsUser vaut 0 ; définir un runAsUser non nul`
                ));
            }
        }
    }
    findings
}

// Image d'un conteneur d'un objet qui utilise un tag `latest` ou pas de tag.
fn latest_tag(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    // Défauts de kube-linter (`latest-tag`, paramètre BlockList) ; un BlockList fourni les remplace.
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
            if matches_any(image, block) {
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

// Sonde absente d'un conteneur (initialisation exclue) : `probe_key` (`livenessProbe`,
// `readinessProbe`) n'est pas renseignée. Une sonde `null` compte comme absente, comme pour kube-linter.
private fn probe_missing_findings(rule, probe_key, label, obj) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            if c.init { continue; }
            if field(c.container, probe_key) == () {
                findings.push(render_lint::finding(
                    rule,
                    "error",
                    obj,
                    c.path,
                    `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : pas de sonde de ${label} (${probe_key}) ; définir une ${probe_key} qui vérifie le conteneur`
                ));
            }
        }
    }
    findings
}

// Vrai quand le port `port` d'une sonde est exposé par le conteneur : un entier doit être le
// `containerPort` d'un port TCP, une chaîne le `name` d'un port TCP. Un protocole absent ou vide vaut
// TCP ; toute autre valeur (UDP, ou `tcp` en minuscules) ne compte pas. Un port absent vaut 0 pour
// kube-linter et n'est jamais exposé.
private fn port_exposed(container, port) {
    let ports = field(container, "ports");
    if type_of(ports) != "array" { return false; }
    for p in ports {
        let proto = field(p, "protocol");
        let have = if type_of(proto) == "string" && proto != "" { proto } else { "TCP" };
        if have != "TCP" { continue; }
        let number = field(p, "containerPort");
        if type_of(port) == "i64" && type_of(number) == "i64" && number == port { return true; }
        let pname = field(p, "name");
        if type_of(port) == "string" && type_of(pname) == "string" && pname == port { return true; }
    }
    false
}

// Port de sonde non exposé : pour chaque conteneur (initialisation exclue) et chaque handler de la
// sonde `probe_key` (`httpGet`, `tcpSocket`, `grpc`) dont le port n'est pas exposé. Un constat par
// handler ; une sonde `exec` ou sans handler ne produit rien.
private fn probe_port_findings(rule, probe_key, label, obj) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            if c.init { continue; }
            let probe = field(c.container, probe_key);
            if type_of(probe) != "map" { continue; }
            for handler in ["httpGet", "tcpSocket", "grpc"] {
                let h = field(probe, handler);
                if type_of(h) != "map" { continue; }
                let port = field(h, "port");
                if port_exposed(c.container, port) { continue; }
                let shown = if type_of(port) == "()" { "absent" } else { `${port}` };
                findings.push(render_lint::finding(
                    rule,
                    "error",
                    obj,
                    c.path + "." + probe_key + "." + handler + ".port",
                    `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : la sonde de ${label} (${handler}) vise le port ${shown}, non exposé par le conteneur ; déclarer ce port dans ports (protocole TCP) ou corriger la sonde`
                ));
            }
        }
    }
    findings
}

// Conteneur (initialisation exclue) sans sonde de vivacité.
fn no_liveness_probe(obj, objs, ctx, params) {
    probe_missing_findings("no-liveness-probe", "livenessProbe", "vivacité", obj)
}

// Conteneur (initialisation exclue) sans sonde de disponibilité.
fn no_readiness_probe(obj, objs, ctx, params) {
    probe_missing_findings("no-readiness-probe", "readinessProbe", "disponibilité", obj)
}

// Sonde de vivacité visant un port que le conteneur n'expose pas.
fn liveness_port(obj, objs, ctx, params) {
    probe_port_findings("liveness-port", "livenessProbe", "vivacité", obj)
}

// Sonde de disponibilité visant un port que le conteneur n'expose pas.
fn readiness_port(obj, objs, ctx, params) {
    probe_port_findings("readiness-port", "readinessProbe", "disponibilité", obj)
}

// Sonde de démarrage visant un port que le conteneur n'expose pas.
fn startup_port(obj, objs, ctx, params) {
    probe_port_findings("startup-port", "startupProbe", "démarrage", obj)
}

// Nombre d'un paramètre entier ou flottant, ou `fallback` si absent ou d'un autre type.
private fn number_param(params, key, fallback) {
    let v = field(params, key);
    if type_of(v) == "i64" { v.to_float() } else if type_of(v) == "f64" { v } else { fallback }
}

// Clés de `resources` retenues par `requirementsType` : `requests`, `limits`, ou les deux pour `any`
// (un constat par clé). Une valeur inconnue ne retient rien : kube-linter refuse la vérification.
private fn requirement_types(value, default_type) {
    let t = if type_of(value) == "string" { value } else { default_type };
    if t == "request" { ["requests"] } else if t == "limit" { ["limits"] } else if t == "any" { ["requests", "limits"] } else { [] }
}

// Demande ou limite de CPU d'un conteneur, en millicores entières (arrondies vers le haut, comme
// `MilliValue()` de kube-linter) ; 0 si absente ; `()` si la forme n'est pas reconnue.
private fn cpu_millis(container, which) {
    let q = field(field(field(container, "resources"), which), "cpu");
    if q == () { return 0.0; }
    if type_of(q) == "i64" { return ceiling(q.to_float() * 1000.0); }
    if type_of(q) == "f64" { return ceiling(q * 1000.0); }
    if type_of(q) != "string" { return (); }
    if regex_is_match("^[0-9]+(\\.[0-9]+)?m$", q) {
        return ceiling(parse_float(q.sub_string(0, q.len() - 1)));
    }
    if regex_is_match("^[0-9]+(\\.[0-9]+)?$", q) { return ceiling(parse_float(q) * 1000.0); }
    ()
}

// Mémoire d'un conteneur en MB (2^20 octets, unité de kube-linter), flottant ; 0 si absente ; `()`
// si la forme n'est pas reconnue. Suffixes binaires (Ki, Mi, Gi, Ti) et décimaux (k, M, G, T, m).
private fn memory_mb(container, which) {
    let q = field(field(field(container, "resources"), which), "memory");
    if q == () { return 0.0; }
    if type_of(q) == "i64" { return q.to_float() / 1048576.0; }
    if type_of(q) == "f64" { return q / 1048576.0; }
    if type_of(q) != "string" { return (); }
    let units = [["Ki", 1024.0], ["Mi", 1048576.0], ["Gi", 1073741824.0], ["Ti", 1099511627776.0], ["k", 1000.0], ["M", 1000000.0], ["G", 1000000000.0], ["T", 1000000000000.0], ["m", 0.001]];
    for u in units {
        if q.ends_with(u[0]) {
            let body = q.sub_string(0, q.len() - u[0].len());
            if !regex_is_match("^[0-9]+(\\.[0-9]+)?$", body) { return (); }
            return parse_float(body) * u[1] / 1048576.0;
        }
    }
    if !regex_is_match("^[0-9]+(\\.[0-9]+)?$", q) { return (); }
    parse_float(q) / 1048576.0
}

// Valeur de la ressource `resource` (`cpu` ou `memory`) sous `which` (`requests` ou `limits`).
private fn requirement_value(container, which, resource) {
    if resource == "cpu" { cpu_millis(container, which) } else { memory_mb(container, which) }
}

// Constats « exigence non bornée » : pour chaque conteneur (initialisation comprise) et chaque type
// retenu, la valeur est signalée quand elle tombe dans l'intervalle fermé [lo ; hi] des paramètres.
// C'est la sémantique exacte de kube-linter : avec les bornes par défaut (0 ; 0), seule une valeur
// absente (lue comme 0) est signalée. Un constat par conteneur et type.
private fn requirement_findings(rule, resource, unit, obj, params, default_type, lo_key, hi_key) {
    import "render_lint" as render_lint;
    let findings = [];
    let types = requirement_types(field(params, "requirementsType"), default_type);
    if types.len() == 0 { return findings; }
    let lo = number_param(params, lo_key, 0.0);
    let hi = number_param(params, hi_key, 0.0);
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            for which in types {
                let v = requirement_value(c.container, which, resource);
                if v == () || v < lo || v > hi { continue; }
                let raw = field(field(field(c.container, "resources"), which), resource);
                let shown = if raw == () { "absente (0)" } else { `"${raw}"` };
                let label = if which == "requests" { "requête" } else { "limite" };
                findings.push(render_lint::finding(
                    rule,
                    "error",
                    obj,
                    c.path + ".resources." + which + "." + resource,
                    `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : ${label} ${resource} ${shown} est dans la plage signalée [${lo} ; ${hi}] ${unit} ; la renseigner hors de cette plage`
                ));
            }
        }
    }
    findings
}

// Demande de CPU absente ou dans la plage signalée. Défauts : requirementsType `request`, bornes 0.
fn unset_cpu_requirements(obj, objs, ctx, params) {
    requirement_findings("unset-cpu-requirements", "cpu", "millicores", obj, params, "request", "lowerBoundMillis", "upperBoundMillis")
}

// Mémoire limite ou demande absente, ou dans la plage signalée (MB, voir `memory_mb`). Défauts :
// requirementsType `limit`, bornes 0.
fn unset_memory_requirements(obj, objs, ctx, params) {
    requirement_findings("unset-memory-requirements", "memory", "MB", obj, params, "limit", "lowerBoundMB", "upperBoundMB")
}

// Port de conteneur (initialisation comprise) dans la plage privilégiée : 0 < containerPort < 1024,
// quel que soit le protocole. `hostPort` n'est pas considéré. Un constat par port.
fn privileged_ports(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            let ports = field(c.container, "ports");
            if type_of(ports) != "array" { continue; }
            let i = 0;
            for p in ports {
                let number = field(p, "containerPort");
                if type_of(number) == "i64" && number > 0 && number < 1024 {
                    findings.push(render_lint::finding(
                        "privileged-ports",
                        "error",
                        obj,
                        `${c.path}.ports[${i}]`,
                        `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : port ${number} mappé dans le conteneur, dans la plage privilégiée (1 à 1023) ; utiliser un port supérieur ou égal à 1024`
                    ));
                }
                i += 1;
            }
        }
    }
    findings
}

// Port SSH (`port`, défaut 22) exposé par un conteneur (initialisation comprise) avec le protocole
// `protocol` (défaut `TCP`). Un port sans `protocol` vaut TCP ; `protocol` vide ou absent du
// paramètre ne filtre pas le protocole. Un constat par port.
fn ssh_port(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    let port = number_param(params, "port", 22.0);
    let wanted = field(params, "protocol");
    let want = if type_of(wanted) == "string" { wanted } else { "TCP" };
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            let ports = field(c.container, "ports");
            if type_of(ports) != "array" { continue; }
            let i = 0;
            for p in ports {
                let number = field(p, "containerPort");
                let proto = field(p, "protocol");
                let have = if type_of(proto) == "string" && proto != "" { proto } else { "TCP" };
                if type_of(number) == "i64" && number.to_float() == port && (want == "" || want == have) {
                    findings.push(render_lint::finding(
                        "ssh-port",
                        "error",
                        obj,
                        `${c.path}.ports[${i}]`,
                        `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : port ${number} (${have}) exposé, port réservé à SSH ; retirer ce port ou le confier à un service vérifié`
                    ));
                }
                i += 1;
            }
        }
    }
    findings
}

// Répertoire hôte monté en écriture : un montage (initialisation comprise) qui référence un volume
// `hostPath` et dont `readOnly` n'est pas `true`. Un `readOnly` qui n'est ni booléen ni absent est
// ignoré (kube-linter ne décode pas ces objets). Un constat par montage.
fn writable_host_mount(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        let hostpaths = #{};
        let volumes = field(field(field(entry, "pod"), "spec"), "volumes");
        if type_of(volumes) == "array" {
            for v in volumes {
                let vname = field(v, "name");
                let path = field(field(v, "hostPath"), "path");
                if type_of(vname) == "string" && type_of(path) == "string" {
                    hostpaths[vname] = path;
                }
            }
        }
        for c in render_lint::containers(entry) {
            let mounts = field(c.container, "volumeMounts");
            if type_of(mounts) != "array" { continue; }
            let j = 0;
            for m in mounts {
                let vname = field(m, "name");
                let ro = field(m, "readOnly");
                let shaped = ro == () || type_of(ro) == "bool";
                if shaped && ro != true && type_of(vname) == "string" && vname in hostpaths {
                    findings.push(render_lint::finding(
                        "writable-host-mount",
                        "error",
                        obj,
                        `${c.path}.volumeMounts[${j}]`,
                        `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : le répertoire hôte "${hostpaths[vname]}" (volume "${vname}") est monté en écriture ; monter avec readOnly: true`
                    ));
                }
                j += 1;
            }
        }
    }
    findings
}

// Chaîne du paramètre `key`, ou `fallback` si le paramètre est absent ou n'est pas une chaîne.
private fn string_param(params, key, fallback) {
    let v = field(params, key);
    if type_of(v) == "string" { v } else { fallback }
}

// Nom du compte de service d'un pod : `serviceAccountName` s'il est renseigné, sinon le champ
// déprécié `serviceAccount` ; chaîne vide si aucun des deux (comme pour kube-linter).
private fn service_account_name(spec) {
    let name = text(field(spec, "serviceAccountName"));
    if name != "" { name } else { text(field(spec, "serviceAccount")) }
}

// Vrai quand la liste `options` de `dnsConfig` contient l'option `key` avec la valeur `value`.
// Une option sans valeur (`()`) ne correspond jamais.
private fn dns_option_found(dns, key, value) {
    let options = field(dns, "options");
    if type_of(options) != "array" { return false; }
    for o in options {
        if field(o, "name") == key && field(o, "value") == value { return true; }
    }
    false
}

// Noms des objets de genre `kind` dans la liste `objs` (tous namespaces confondus : kube-linter
// ne rapproche une référence de son objet que par le nom et le genre).
private fn known_names(objs, kind) {
    let names = [];
    if type_of(objs) != "array" { return names; }
    for o in objs {
        if field(o, "kind") == kind {
            let n = text(field(field(o, "metadata"), "name"));
            if n != "" { names.push(n); }
        }
    }
    names
}

// Compte de service par défaut : le nom du compte de service du pod correspond à `serviceAccount`
// (défaut `^(|default)$` : absent, vide ou `default`). Un constat par pod.
fn default_service_account(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let pattern = string_param(params, "serviceAccount", "^(|default)$");
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        let spec = field(field(entry, "pod"), "spec");
        if type_of(spec) != "map" { continue; }
        let sa = service_account_name(spec);
        if regex_is_match(pattern, sa) {
            let shown = if sa == "" { "absent" } else { `"${sa}"` };
            findings.push(render_lint::finding(
                "default-service-account",
                "error",
                obj,
                pod_spec_path(entry) + ".serviceAccountName",
                `${kind} "${name}" : compte de service ${shown} (défaut) ; créer un compte dédié et le nommer par serviceAccountName`
            ));
        }
    }
    findings
}

// Option DNS attendue absente de `dnsConfig` (défaut : option `ndots` de valeur `2`). Une option
// `dnsConfig` absente compte comme absente ; une `dnsConfig` de forme inconnue n'est pas signalée.
// Un constat par pod.
fn dnsconfig_options(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let key = string_param(params, "Key", "ndots");
    let value = string_param(params, "Value", "2");
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        let spec = field(field(entry, "pod"), "spec");
        if type_of(spec) != "map" { continue; }
        // Structure non reconnue (dnsConfig ou options qui ne sont ni un objet ni un tableau) :
        // rien à dire, kube-linter ne décode pas ce pod.
        let dns = field(spec, "dnsConfig");
        if dns != () && type_of(dns) != "map" { continue; }
        let options = field(dns, "options");
        if options != () && type_of(options) != "array" { continue; }
        if dns_option_found(dns, key, value) { continue; }
        findings.push(render_lint::finding(
            "dnsconfig-options",
            "error",
            obj,
            pod_spec_path(entry) + ".dnsConfig",
            `${kind} "${name}" : l'option DNS "${key}" de valeur "${value}" est absente de dnsConfig ; ajouter ${key}: "${value}" à dnsConfig.options`
        ));
    }
    findings
}

// Classe de priorité du pod hors de la liste `acceptedPriorityClassNames` (défaut : les deux
// classes système). Une classe absente ou vide n'est pas signalée. Un constat par pod.
fn priority_class_name(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let accepted = list_param(params, "acceptedPriorityClassNames", ["system-cluster-critical", "system-node-critical"]);
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        let spec = field(field(entry, "pod"), "spec");
        if type_of(spec) != "map" { continue; }
        let pc = text(field(spec, "priorityClassName"));
        if pc != "" && !contains_value(accepted, pc) {
            findings.push(render_lint::finding(
                "priority-class-name",
                "error",
                obj,
                pod_spec_path(entry) + ".priorityClassName",
                `${kind} "${name}" : classe de priorité "${pc}" non acceptée ; utiliser l'une des classes acceptées (${accepted})`
            ));
        }
    }
    findings
}

// Politique de redémarrage du pod différente de `Always` ou `OnFailure`, absente comprise (kube-linter
// n'accepte pas une politique vide). Un constat par pod.
fn restart_policy(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        let spec = field(field(entry, "pod"), "spec");
        if type_of(spec) != "map" { continue; }
        let rp = text(field(spec, "restartPolicy"));
        if rp != "Always" && rp != "OnFailure" {
            let shown = if rp == "" { "absente" } else { `"${rp}"` };
            findings.push(render_lint::finding(
                "restart-policy",
                "error",
                obj,
                pod_spec_path(entry) + ".restartPolicy",
                `${kind} "${name}" : politique de redémarrage ${shown} ; utiliser "Always" ou "OnFailure"`
            ));
        }
    }
    findings
}

// Variable d'environnement déclarée plusieurs fois dans un même conteneur (initialisation comprise).
// Un constat par nom dupliqué et par conteneur, quel que soit le nombre de répétitions.
fn duplicate_env_var(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            let env = field(c.container, "env");
            if type_of(env) != "array" { continue; }
            let reported = [];
            for e in env {
                let n = field(e, "name");
                if type_of(n) != "string" || n in reported { continue; }
                let count = 0;
                for other in env {
                    if field(other, "name") == n { count += 1; }
                }
                if count > 1 {
                    reported.push(n);
                    findings.push(render_lint::finding(
                        "duplicate-env-var",
                        "error",
                        obj,
                        c.path + ".env",
                        `conteneur "${field(c.container, "name")}" de ${kind} "${name}" : la variable "${n}" est déclarée ${count} fois ; n'en garder qu'une seule`
                    ));
                }
            }
        }
    }
    findings
}

// Référence `valueFrom` (secretKeyRef ou configMapKeyRef) vers un Secret ou un ConfigMap absent de
// la liste d'objets (nom exact, genre compris), ou dont le nom correspond à un motif de
// IgnoredSecrets / IgnoredConfigMaps (expressions régulières non ancrées, comme kube-linter). Un constat par référence
// d'env (initialisation comprise) ; envFrom et volumes ne sont pas examinés.
fn env_value_from(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let known_secrets = known_names(objs, "Secret");
    let known_configmaps = known_names(objs, "ConfigMap");
    let ignored_secrets = list_param(params, "IgnoredSecrets", []);
    let ignored_configmaps = list_param(params, "IgnoredConfigMaps", []);
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            let env = field(c.container, "env");
            if type_of(env) != "array" { continue; }
            let cname = field(c.container, "name");
            let i = 0;
            for e in env {
                let vf = field(e, "valueFrom");
                let sref = field(vf, "secretKeyRef");
                let sname = field(sref, "name");
                if type_of(sname) == "string" && !(sname in known_secrets) && !matches_any(sname, ignored_secrets) {
                    findings.push(render_lint::finding(
                        "env-value-from",
                        "error",
                        obj,
                        `${c.path}.env[${i}].valueFrom.secretKeyRef`,
                        `conteneur "${cname}" de ${kind} "${name}" : le Secret "${sname}" référencé par la variable "${field(e, "name")}" n'est pas dans la liste d'objets ; créer ce Secret ou corriger la référence`
                    ));
                }
                let cref = field(vf, "configMapKeyRef");
                let cmname = field(cref, "name");
                if type_of(cmname) == "string" && !(cmname in known_configmaps) && !matches_any(cmname, ignored_configmaps) {
                    findings.push(render_lint::finding(
                        "env-value-from",
                        "error",
                        obj,
                        `${c.path}.env[${i}].valueFrom.configMapKeyRef`,
                        `conteneur "${cname}" de ${kind} "${name}" : le ConfigMap "${cmname}" référencé par la variable "${field(e, "name")}" n'est pas dans la liste d'objets ; créer ce ConfigMap ou corriger la référence`
                    ));
                }
                i += 1;
            }
        }
    }
    findings
}

// Variable d'environnement dont le nom correspond à `name` (défaut `(?i).*secret.*`) et dont la
// valeur littérale correspond à `value` (défaut `.+`, donc non vide) : un secret en clair. La
// valeur n'est jamais recopiée dans le message. Un constat par variable (initialisation comprise).
fn env_var_secret(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let name_pattern = string_param(params, "name", "(?i).*secret.*");
    let value_pattern = string_param(params, "value", ".+");
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            let env = field(c.container, "env");
            if type_of(env) != "array" { continue; }
            let cname = field(c.container, "name");
            let i = 0;
            for e in env {
                let n = field(e, "name");
                let v = field(e, "value");
                if type_of(n) == "string" && type_of(v) == "string"
                    && regex_is_match(name_pattern, n) && regex_is_match(value_pattern, v) {
                    findings.push(render_lint::finding(
                        "env-var-secret",
                        "error",
                        obj,
                        `${c.path}.env[${i}].value`,
                        `conteneur "${cname}" de ${kind} "${name}" : la variable "${n}" porte un secret en clair ; monter le secret en fichier ou le lire par valueFrom.secretKeyRef`
                    ));
                }
                i += 1;
            }
        }
    }
    findings
}

// Variable d'environnement lue depuis un secret (`valueFrom.secretKeyRef`), initialisation comprise.
// Un constat par variable. envFrom n'est pas examiné.
fn read_secret_from_env_var(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    let findings = [];
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    for entry in render_lint::pod_templates(obj) {
        for c in render_lint::containers(entry) {
            let env = field(c.container, "env");
            if type_of(env) != "array" { continue; }
            let cname = field(c.container, "name");
            let i = 0;
            for e in env {
                let sref = field(field(e, "valueFrom"), "secretKeyRef");
                if type_of(sref) == "map" {
                    findings.push(render_lint::finding(
                        "read-secret-from-env-var",
                        "error",
                        obj,
                        `${c.path}.env[${i}].valueFrom.secretKeyRef`,
                        `conteneur "${cname}" de ${kind} "${name}" : la variable "${text(field(e, "name"))}" lit un secret depuis l'environnement ; monter le secret en fichier à la place`
                    ));
                }
                i += 1;
            }
        }
    }
    findings
}
