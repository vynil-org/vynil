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
