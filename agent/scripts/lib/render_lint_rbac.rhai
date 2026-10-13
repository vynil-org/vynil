// Attribution : les vérifications de ce module reprennent la logique de kube-linter
// (https://github.com/stackrox/kube-linter, licence Apache 2.0) : noms, paramètres, valeurs par
// défaut et sémantique mesurée sur v0.8.3. Voir /NOTICE et /third_party/kube-linter/LICENSE.
// Thème « rbac » des vérifications portées de kube-linter (spec : render_lint_rbac.sdd).
// Chaque vérification est pure : elle ne lit que ses arguments et retourne un tableau de
// constats, vide quand rien n'est à signaler. `render_lint` est importé dans le corps de chaque
// fonction (et non en tête) : le noyau importe ce module, un import en tête ferait un cycle.
//
// Périmètre des genres (mesuré sur kube-linter v0.8.3) : seuls les objets
// rbac.authorization.k8s.io/v1 sont examinés. Un Role, une liaison ou un ClusterRole en v1beta1 ne
// donne jamais de constat, et un groupe tiers homonyme (`rbac.example.com/v1`) est ignoré. Ce
// périmètre déroge à la règle « toutes versions confondues » des charges de travail : c'est la
// mesure qui le fixe.
//
// Résolution d'une liaison (mesurée) : un RoleBinding désigne un Role de MÊME namespace par son
// seul nom ; un ClusterRoleBinding désigne un ClusterRole par son seul nom, quel que soit son
// namespace. Le genre (`roleRef.kind`) et le groupe (`roleRef.apiGroup`) du roleRef ne comptent
// pas pour ces vérifications d'accès, ni pour access-to-*.

// Lit une clé d'une carte ; `()` si la valeur n'est pas une carte ou si la clé est absente.
private fn field(m, key) {
    if type_of(m) == "map" && key in m { m[key] } else { () }
}

// Chaîne, ou chaîne vide si la valeur n'en est pas une.
private fn text(v) {
    if type_of(v) == "string" { v } else { "" }
}

// Éléments chaînes d'un tableau ; `[]` si la valeur n'est pas un tableau.
private fn strings(v) {
    let out = [];
    if type_of(v) == "array" {
        for x in v {
            if type_of(x) == "string" { out.push(x); }
        }
    }
    out
}

// Paramètre tableau, ou `fallback` si absent ou qui n'est pas un tableau.
private fn list_param(params, key, fallback) {
    let v = field(params, key);
    if type_of(v) == "array" { v } else { fallback }
}

// Vrai quand `value` correspond à au moins un motif (regex) de `patterns`.
private fn matches_any(value, patterns) {
    for pattern in patterns {
        if type_of(pattern) == "string" && regex_is_match(pattern, value) { return true; }
    }
    false
}

// Namespace et nom d'un objet (chaîne vide si absents).
private fn ns_of(obj) {
    text(field(field(obj, "metadata"), "namespace"))
}

private fn name_of(obj) {
    text(field(field(obj, "metadata"), "name"))
}

// Vrai quand l'objet est de l'un des genres `kinds` en rbac.authorization.k8s.io/v1.
private fn rbac_v1(obj, kinds) {
    import "render_lint" as render_lint;
    render_lint::kind_of_group(obj, kinds, "rbac.authorization.k8s.io")
        && text(field(obj, "apiVersion")) == "rbac.authorization.k8s.io/v1"
}

// Constats des éléments `*` de la liste `key` (`resources` ou `verbs`) de la règle d'indice `i` :
// un constat par occurrence, chemin `rules[i].key[j]`. `what` nomme la liste dans le message.
private fn wildcard_in(obj, i, rule, key, what) {
    import "render_lint" as render_lint;
    let out = [];
    let raw = field(rule, key);
    if type_of(raw) != "array" { return out; }
    let j = 0;
    for v in raw {
        if v == "*" {
            out.push(render_lint::finding(
                "wildcard-in-rules",
                "error",
                obj,
                `rules[${i}].${key}[${j}]`,
                `${text(field(obj, "kind"))} "${name_of(obj)}" : joker "*" dans les ${what} de la règle ${i} ; lister les ${what} explicitement`
            ));
        }
        j += 1;
    }
    out
}

// Vérification wildcard-in-rules : un constat par élément `*` de `resources` ou de `verbs`, dans
// les règles d'un Role ou d'un ClusterRole (v1). `apiGroups` et `nonResourceURLs` ne comptent pas.
fn wildcard_in_rules(obj, idx, ctx, params) {
    if !rbac_v1(obj, ["Role", "ClusterRole"]) { return []; }
    let rules = field(obj, "rules");
    if type_of(rules) != "array" { return []; }
    let out = [];
    let i = 0;
    for rule in rules {
        if type_of(rule) == "map" {
            for f in wildcard_in(obj, i, rule, "resources", "ressources") { out.push(f); }
            for f in wildcard_in(obj, i, rule, "verbs", "verbes") { out.push(f); }
        }
        i += 1;
    }
    out
}

// Vérification cluster-admin-role-binding : un ClusterRoleBinding dont le roleRef est le
// ClusterRole `cluster-admin` (le genre doit être ClusterRole : mesuré). Ni les sujets (absents
// compris), ni le groupe du roleRef, ni le namespace ne comptent.
fn cluster_admin_role_binding(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    if !rbac_v1(obj, ["ClusterRoleBinding"]) { return []; }
    let rref = field(obj, "roleRef");
    if text(field(rref, "kind")) != "ClusterRole" || text(field(rref, "name")) != "cluster-admin" {
        return [];
    }
    let name = name_of(obj);
    [render_lint::finding(
        "cluster-admin-role-binding",
        "error",
        obj,
        "roleRef",
        `ClusterRoleBinding "${name}" : lie le rôle cluster-admin ; créer un rôle dédié aux seuls droits nécessaires à ce sujet`
    )]
}

// Rôle désigné par une liaison, ou `()`. RoleBinding : le Role de même namespace et de même nom.
// ClusterRoleBinding : le ClusterRole de même nom, quel que soit son namespace. Premier trouvé.
private fn bound_role(obj, idx) {
    let rref = field(obj, "roleRef");
    let wanted = text(field(rref, "name"));
    if wanted == "" { return (); }
    if rbac_v1(obj, ["RoleBinding"]) {
        idx.rbac_roles.first(`Role|${ns_of(obj)}|${wanted}`)
    } else {
        idx.rbac_roles.first(`ClusterRole||${wanted}`)
    }
}

// Constat unique d'une liaison dont le rôle désigné a une règle où un verbe ET une ressource
// concordent (dans la même règle). Une paire est `[verbe] access to [ressource]`, comme l'outil.
// `*` concorde avec tout ; sinon la liste de motifs (regex) du paramètre, ou la liste par défaut.
private fn access_findings(check, obj, idx, params, resources, verbs) {
    import "render_lint" as render_lint;
    if !rbac_v1(obj, ["RoleBinding", "ClusterRoleBinding"]) { return []; }
    let role = bound_role(obj, idx);
    if role == () { return []; }
    let verb_patterns = list_param(params, "verbs", verbs);
    let resource_patterns = list_param(params, "resources", resources);
    let rules = field(role, "rules");
    if type_of(rules) != "array" { return []; }
    let detail = "";
    let count = 0;
    for rule in rules {
        if type_of(rule) != "map" { continue; }
        for v in strings(field(rule, "verbs")) {
            for r in strings(field(rule, "resources")) {
                let verb_ok = v == "*" || matches_any(v, verb_patterns);
                let resource_ok = r == "*" || matches_any(r, resource_patterns);
                if verb_ok && resource_ok {
                    if count > 0 { detail += ", "; }
                    detail += `[${v}] access to [${r}]`;
                    count += 1;
                }
            }
        }
    }
    if count == 0 { return []; }
    let role_name = name_of(role);
    [render_lint::finding(
        check,
        "error",
        obj,
        "roleRef",
        `${text(field(obj, "kind"))} "${name_of(obj)}" lie le rôle "${role_name}" qui accorde ${detail} ; retirer ces droits ou restreindre verbes et ressources`
    )]
}

// Vérification access-to-create-pods : une liaison dont le rôle permet de créer des pods ou des
// charges de travail (ressources et verbes par défaut de kube-linter). `cronjob` est au singulier
// tel quel : mesuré, `cronjobs` (pluriel) n'est pas concerné.
fn access_to_create_pods(obj, idx, ctx, params) {
    access_findings(
        "access-to-create-pods",
        obj,
        idx,
        params,
        ["^pods$", "^deployments$", "^statefulsets$", "^replicasets$", "^cronjob$", "^jobs$", "^daemonsets$"],
        ["^create$"]
    )
}

// Vérification access-to-secrets : une liaison dont le rôle donne accès aux secrets. Le motif
// `^*$` de la liste par défaut concorde avec tout verbe (mesuré : `update`, `foo` et `""` aussi).
fn access_to_secrets(obj, idx, ctx, params) {
    access_findings(
        "access-to-secrets",
        obj,
        idx,
        params,
        ["^secrets$"],
        ["^get$", "^list$", "^delete$", "^create$", "^watch$", "^*$"]
    )
}

// Vrai quand un ServiceAccount v1 nommé `name` existe dans le namespace `ns` de la liste.
private fn has_service_account(idx, ns, name) {
    idx.rbac_service_accounts.has(`${ns}|${name}`)
}

// Vérification non-existent-service-account : un gabarit de pod (Pod compris) dont le compte de
// service n'existe pas dans la liste, dans le namespace de la charge de travail. Le champ
// `serviceAccountName`, s'il est non vide, l'emporte sur le champ déprécié `serviceAccount`. Un
// compte vide ou `default` n'est jamais signalé (mesuré).
fn non_existent_service_account(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    let findings = [];
    for entry in render_lint::pod_templates(obj) {
        let spec = field(field(entry, "pod"), "spec");
        let key = "serviceAccountName";
        let account = text(field(spec, key));
        if account == "" {
            key = "serviceAccount";
            account = text(field(spec, key));
        }
        if account == "" || account == "default" { continue; }
        let ns = ns_of(obj);
        if has_service_account(idx, ns, account) { continue; }
        let base = text(field(entry, "path"));
        let prefix = if base == "" { "spec" } else { base + ".spec" };
        findings.push(render_lint::finding(
            "non-existent-service-account",
            "error",
            obj,
            `${prefix}.${key}`,
            `${text(field(obj, "kind"))} "${name_of(obj)}" : ${key} "${account}" ne correspond à aucun ServiceAccount du namespace "${ns}" ; créer ce ServiceAccount ou référencer un compte existant`
        ));
    }
    findings
}

// Entrées d'index de ce thème, construites une fois par `render_lint::index` :
//   rbac_roles            : `Role|namespace|nom` et `ClusterRole||nom` -> le rôle (le premier de
//                           la liste fait foi, comme le parcours d'origine) ;
//   rbac_service_accounts : `namespace|nom` -> vrai pour un ServiceAccount du noyau.
fn index(objs) {
    import "render_lint" as render_lint;
    let roles = multimap();
    let accounts = multimap();
    for o in objs {
        if rbac_v1(o, ["Role"]) {
            roles.put(`Role|${ns_of(o)}|${name_of(o)}`, o);
        } else if rbac_v1(o, ["ClusterRole"]) {
            roles.put(`ClusterRole||${name_of(o)}`, o);
        } else if render_lint::kind_of_group(o, ["ServiceAccount"], "") {
            accounts.put(`${ns_of(o)}|${name_of(o)}`, true);
        }
    }
    #{ rbac_roles: roles, rbac_service_accounts: accounts }
}
