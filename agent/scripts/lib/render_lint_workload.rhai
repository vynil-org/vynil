// Thème « workload » des vérifications portées de kube-linter (spec : render_lint_workload.sdd).
// Chaque vérification est pure : elle ne lit que ses arguments et retourne un tableau de
// constats, vide quand rien n'est à signaler. `render_lint` est importé dans le corps de chaque
// fonction (et non en tête) : le noyau importe ce module, un import en tête ferait un cycle.
//
// Périmètre des genres : kube-linter n'applique ces vérifications qu'aux objets du groupe d'API
// attendu pour leur genre (`apps` pour Deployment, StatefulSet, DaemonSet, ReplicaSet ; `batch`
// pour Job, CronJob ; le groupe du noyau pour Pod, ReplicationController, Service). Un Deployment
// `extensions/v1beta1` est donc silencieux pour ces vérifications et signalé par
// `no-extensions-v1beta`. Les versions d'un même groupe ne sont pas distinguées.

// Lit une clé d'une carte ; `()` si la valeur n'est pas une carte ou si la clé est absente.
private fn field(m, key) {
    if type_of(m) == "map" && key in m { m[key] } else { () }
}

// Chaîne, ou chaîne vide si la valeur n'en est pas une.
private fn text(v) {
    if type_of(v) == "string" { v } else { "" }
}

// Paramètre entier `key`, ou `fallback` si absent ou non entier.
private fn int_param(params, key, fallback) {
    let v = field(params, key);
    if type_of(v) == "i64" { v } else { fallback }
}

// Paramètre chaîne `key`, ou `fallback` si absent ou non chaîne.
private fn string_param(params, key, fallback) {
    let v = field(params, key);
    if type_of(v) == "string" { v } else { fallback }
}

// Groupe d'une apiVersion : `apps` pour `apps/v1`, chaîne vide pour `v1`.
private fn group_of(api) {
    let parts = api.split("/");
    if parts.len() == 2 { parts[0] } else { "" }
}

// Version d'une apiVersion : `v1` pour `apps/v1` comme pour `v1`.
private fn version_of(api) {
    let parts = api.split("/");
    parts[parts.len() - 1]
}

// Vrai quand l'objet a l'un des genres `kinds` dans le groupe d'API `group`.
private fn kind_of_group(obj, kinds, group) {
    let kind = text(field(obj, "kind"));
    (kind in kinds) && (group_of(text(field(obj, "apiVersion"))) == group)
}

// Objets de la famille DeploymentLike de kube-linter : ceux qui embarquent un gabarit de pod.
private fn deployment_like(obj) {
    kind_of_group(obj, ["Deployment", "StatefulSet", "DaemonSet", "ReplicaSet"], "apps")
        || kind_of_group(obj, ["Job", "CronJob"], "batch")
        || kind_of_group(obj, ["Pod", "ReplicationController"], "")
}

// Objets qui portent un nombre de réplicas (`spec.replicas`).
private fn replicated(obj) {
    kind_of_group(obj, ["Deployment", "StatefulSet", "ReplicaSet"], "apps")
        || kind_of_group(obj, ["ReplicationController"], "")
}

// Nombre de réplicas : `spec.replicas`, 1 quand le champ est absent (défaut de Kubernetes).
// Spec absente ou non carte : `()`, pas de réplicas à évaluer.
private fn replicas_of(obj) {
    let spec = field(obj, "spec");
    if type_of(spec) != "map" { return (); }
    let r = field(spec, "replicas");
    if r == () { 1 } else { r }
}

// Spec du pod d'une entrée de `pod_templates`.
private fn pod_spec(entry) {
    field(field(entry, "pod"), "spec")
}

// Chemin de la spec du pod d'une entrée de `pod_templates` : `spec` ou `spec.template.spec`.
private fn pod_spec_path(entry) {
    let base = text(field(entry, "path"));
    if base == "" { "spec" } else { base + ".spec" }
}

// Vrai quand la valeur est présente mais n'est pas une carte (forme inattendue).
private fn malformed(v) {
    v != () && type_of(v) != "map"
}

// Vrai quand la valeur est présente mais n'est pas un tableau (forme inattendue).
private fn malformed_list(v) {
    v != () && type_of(v) != "array"
}

// Vrai quand une liste d'affinité contient au moins une règle.
private fn has_terms(v) {
    type_of(v) == "array" && v.len() > 0
}

// Vrai quand une expression de sélecteur (`matchExpressions`) est satisfaite par les labels.
// Opérateur inconnu : pas de constat (la sémantique n'est pas définie).
private fn expression_matches(e, labels) {
    let key = text(field(e, "key"));
    let op = text(field(e, "operator"));
    let values = field(e, "values");
    let vals = if type_of(values) == "array" { values } else { [] };
    let present = key in labels;
    let value = if present { labels[key] } else { () };
    if op == "In" {
        present && (value in vals)
    } else if op == "NotIn" {
        !(present && (value in vals))
    } else if op == "Exists" {
        present
    } else if op == "DoesNotExist" {
        !present
    } else {
        true
    }
}

// Réplicas inférieurs à `minReplicas` (défaut 3) sur un Deployment, StatefulSet, ReplicaSet ou
// ReplicationController.
fn minimum_three_replicas(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    if !replicated(obj) { return []; }
    let min = int_param(params, "minReplicas", 3);
    let replicas = replicas_of(obj);
    if type_of(replicas) != "i64" || replicas >= min { return []; }
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    [render_lint::finding(
        "minimum-three-replicas",
        "error",
        obj,
        "spec.replicas",
        `${kind} "${name}" : ${replicas} réplica(s), minimum ${min} ; porter spec.replicas à au moins ${min}`
    )]
}

// Job autonome sans `ttlSecondsAfterFinished`, ou CronJob dont le gabarit de Job en définit un
// (ils entrent en conflit avec successfulJobsHistoryLimit). Une valeur nulle compte comme absente.
fn job_ttl_seconds_after_finished(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    if type_of(field(obj, "spec")) != "map" { return []; }
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    if kind_of_group(obj, ["Job"], "batch") {
        if field(field(obj, "spec"), "ttlSecondsAfterFinished") != () { return []; }
        return [render_lint::finding(
            "job-ttl-seconds-after-finished",
            "error",
            obj,
            "spec.ttlSecondsAfterFinished",
            `Job "${name}" : ttlSecondsAfterFinished absent ; le définir pour que le Job soit nettoyé après exécution`
        )];
    }
    if kind_of_group(obj, ["CronJob"], "batch") {
        let ttl = field(field(field(field(obj, "spec"), "jobTemplate"), "spec"), "ttlSecondsAfterFinished");
        if ttl == () { return []; }
        return [render_lint::finding(
            "job-ttl-seconds-after-finished",
            "error",
            obj,
            "spec.jobTemplate.spec.ttlSecondsAfterFinished",
            `CronJob "${name}" : ttlSecondsAfterFinished défini dans le gabarit du Job ; le retirer, il entre en conflit avec successfulJobsHistoryLimit`
        )];
    }
    []
}

// Champ déprécié `serviceAccount` du pod : signalé quand il est renseigné sans `serviceAccountName`,
// ou quand il diffère de `serviceAccountName`. Des valeurs identiques ne sont pas signalées.
fn deprecated_service_account_field(obj, objs, ctx, params) {
    import "render_lint" as render_lint;
    if !deployment_like(obj) { return []; }
    let templates = render_lint::pod_templates(obj);
    if templates.len() == 0 { return []; }
    let spec = pod_spec(templates[0]);
    let sa = field(spec, "serviceAccount");
    if type_of(sa) != "string" || sa == "" { return []; }
    let san = text(field(spec, "serviceAccountName"));
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    let path = pod_spec_path(templates[0]) + ".serviceAccount";
    if san == "" {
        return [render_lint::finding(
            "deprecated-service-account-field",
            "error",
            obj,
            path,
            `${kind} "${name}" : serviceAccount "${sa}" est déprécié ; utiliser serviceAccountName à la place`
        )];
    }
    if san == sa { return []; }
    [render_lint::finding(
        "deprecated-service-account-field",
        "error",
        obj,
        path,
        `${kind} "${name}" : serviceAccount "${sa}" et serviceAccountName "${san}" diffèrent ; supprimer serviceAccount et garder serviceAccountName`
    )]
}