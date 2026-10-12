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
fn wildcard_in_rules(obj, objs, ctx, params) {
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
fn cluster_admin_role_binding(obj, objs, ctx, params) {
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