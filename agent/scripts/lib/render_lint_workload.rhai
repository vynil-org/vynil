// Attribution : les vérifications de ce module reprennent la logique de kube-linter
// (https://github.com/stackrox/kube-linter, licence Apache 2.0) : noms, paramètres, valeurs par
// défaut et sémantique mesurée sur v0.8.3. Voir /NOTICE et /third_party/kube-linter/LICENSE.
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

// Groupe d'une apiVersion, et identification d'un objet par groupe et kind : délégués au noyau
// (render_lint::api_group_of, render_lint::kind_of_group), seul endroit où cette règle vit.
private fn group_of(api) {
    import "render_lint" as render_lint;
    render_lint::api_group_of(api)
}

// Version d'une apiVersion : `v1` pour `apps/v1` comme pour `v1`.
private fn version_of(api) {
    let parts = api.split("/");
    parts[parts.len() - 1]
}

// Vrai quand l'objet a l'un des genres `kinds` dans le groupe d'API `group`.
private fn kind_of_group(obj, kinds, group) {
    import "render_lint" as render_lint;
    render_lint::kind_of_group(obj, kinds, group)
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
fn minimum_three_replicas(obj, idx, ctx, params) {
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

// Réplicas au moins égaux à `minReplicas` (défaut 2) sans anti-affinité entre pods : `podAntiAffinity`
// absent, ou présent sans aucune règle `preferred` ni `required`.
fn no_anti_affinity(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    if !replicated(obj) { return []; }
    let min = int_param(params, "minReplicas", 2);
    let replicas = replicas_of(obj);
    if type_of(replicas) != "i64" || replicas < min { return []; }
    let templates = render_lint::pod_templates(obj);
    if templates.len() == 0 { return []; }
    let base = pod_spec_path(templates[0]);
    let spec = pod_spec(templates[0]);
    if type_of(spec) != "map" || malformed(field(spec, "affinity")) { return []; }
    let affinity = field(spec, "affinity");
    let anti = field(affinity, "podAntiAffinity");
    if malformed(anti) { return []; }
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    if anti == () {
        return [render_lint::finding(
            "no-anti-affinity",
            "error",
            obj,
            base + ".affinity",
            `${kind} "${name}" : ${replicas} réplicas sans anti-affinité entre pods ; définir spec.affinity.podAntiAffinity avec une règle preferred ou required`
        )];
    }
    if has_terms(field(anti, "preferredDuringSchedulingIgnoredDuringExecution"))
        || has_terms(field(anti, "requiredDuringSchedulingIgnoredDuringExecution")) {
        return [];
    }
    [render_lint::finding(
        "no-anti-affinity",
        "error",
        obj,
        base + ".affinity.podAntiAffinity",
        `${kind} "${name}" : ${replicas} réplicas, podAntiAffinity ne définit ni preferred ni required ; ajouter au moins une règle de préférence ou d'obligation`
    )]
}

// API obsolète : objet dont le groupe vaut `group` (défaut `extensions`) et dont la version
// correspond à `version` (défaut `v1beta.+`), quel que soit son genre.
fn no_extensions_v1beta(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    let group = string_param(params, "group", "extensions");
    let pattern = string_param(params, "version", "v1beta.+");
    let api = text(field(obj, "apiVersion"));
    if group_of(api) != group || !regex_is_match(pattern, version_of(api)) { return []; }
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    [render_lint::finding(
        "no-extensions-v1beta",
        "error",
        obj,
        "apiVersion",
        `${kind} "${name}" : apiVersion ${api} est obsolète ; migrer vers une version stable du même genre (par exemple apps/v1 pour un Deployment)`
    )]
}

// Workload sans affinité de nœud : `nodeAffinity` absent du champ `affinity` du pod. Une affinité
// vide (`nodeAffinity: {}`) compte comme définie.
fn no_node_affinity(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    if !deployment_like(obj) { return []; }
    let templates = render_lint::pod_templates(obj);
    if templates.len() == 0 { return []; }
    let spec = pod_spec(templates[0]);
    if type_of(spec) != "map" || malformed(field(spec, "affinity")) { return []; }
    if field(field(spec, "affinity"), "nodeAffinity") != () { return []; }
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    [render_lint::finding(
        "no-node-affinity",
        "error",
        obj,
        pod_spec_path(templates[0]) + ".affinity",
        `${kind} "${name}" : aucune affinité de nœud ; définir spec.affinity.nodeAffinity pour choisir les nœuds d'exécution`
    )]
}

// Stratégie de mise à jour absente ou dont le type ne correspond pas à `strategyTypeRegex`
// (défaut `^(RollingUpdate|Rolling)$`). Un Deployment lit `spec.strategy`, un StatefulSet ou un
// DaemonSet lit `spec.updateStrategy` ; les autres genres ne sont pas concernés.
fn no_rolling_update_strategy(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    let field_name = if kind_of_group(obj, ["Deployment"], "apps") {
        "strategy"
    } else if kind_of_group(obj, ["StatefulSet", "DaemonSet"], "apps") {
        "updateStrategy"
    } else {
        ""
    };
    if field_name == "" { return []; }
    let regex = string_param(params, "strategyTypeRegex", "^(RollingUpdate|Rolling)$");
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    let path = "spec." + field_name + ".type";
    let spec = field(obj, "spec");
    if type_of(spec) != "map" || malformed(field(spec, field_name)) { return []; }
    let stype = field(field(spec, field_name), "type");
    if type_of(stype) != "string" || stype == "" {
        return [render_lint::finding(
            "no-rolling-update-strategy",
            "error",
            obj,
            path,
            `${kind} "${name}" : aucun type de stratégie (${path}) ; définir un type qui corresponde à ${regex}`
        )];
    }
    if regex_is_match(regex, stype) { return []; }
    [render_lint::finding(
        "no-rolling-update-strategy",
        "error",
        obj,
        path,
        `${kind} "${name}" : stratégie ${stype} (${path}) ; utiliser un type qui corresponde à ${regex}, par exemple RollingUpdate`
    )]
}

// Job autonome sans `ttlSecondsAfterFinished`, ou CronJob dont le gabarit de Job en définit un
// (ils entrent en conflit avec successfulJobsHistoryLimit). Une valeur nulle compte comme absente.
fn job_ttl_seconds_after_finished(obj, idx, ctx, params) {
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

// Sélecteur qui ne correspond pas aux labels du gabarit. Pour Deployment, StatefulSet, DaemonSet et
// ReplicaSet, un sélecteur absent ou vide est signalé ; pour Job, un sélecteur absent est normal
// (Kubernetes le génère). Un ReplicationController (sélecteur en simple carte), un CronJob et un
// Pod ne sont pas vérifiés.
fn mismatching_selector(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    let is_job = kind_of_group(obj, ["Job"], "batch");
    if !(is_job || kind_of_group(obj, ["Deployment", "StatefulSet", "DaemonSet", "ReplicaSet"], "apps")) {
        return [];
    }
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    let spec = field(obj, "spec");
    if type_of(spec) != "map" || malformed(field(spec, "template")) { return []; }
    let selector = field(spec, "selector");
    if malformed(selector) { return []; }
    let match_labels = field(selector, "matchLabels");
    let exprs = field(selector, "matchExpressions");
    if malformed(match_labels) || malformed_list(exprs) { return []; }
    let has_labels = type_of(match_labels) == "map" && match_labels.len() > 0;
    let has_exprs = type_of(exprs) == "array" && exprs.len() > 0;
    if !has_labels && !has_exprs {
        if is_job { return []; }
        return [render_lint::finding(
            "mismatching-selector",
            "error",
            obj,
            "spec.selector",
            `${kind} "${name}" : aucun sélecteur ; définir spec.selector.matchLabels correspondant aux labels du gabarit`
        )];
    }
    let labels = field(field(field(field(obj, "spec"), "template"), "metadata"), "labels");
    if type_of(labels) != "map" { labels = #{}; }
    let ok = true;
    if has_labels {
        for k in match_labels.keys() {
            if !(k in labels) || labels[k] != match_labels[k] { ok = false; }
        }
    }
    if has_exprs {
        for e in exprs {
            if !expression_matches(e, labels) { ok = false; }
        }
    }
    if ok { return []; }
    [render_lint::finding(
        "mismatching-selector",
        "error",
        obj,
        "spec.selector",
        `${kind} "${name}" : le sélecteur (spec.selector) ne correspond pas aux labels de spec.template.metadata.labels ; aligner le sélecteur sur le gabarit`
    )]
}

// Champ déprécié `serviceAccount` du pod : signalé quand il est renseigné sans `serviceAccountName`,
// ou quand il diffère de `serviceAccountName`. Des valeurs identiques ne sont pas signalées.
fn deprecated_service_account_field(obj, idx, ctx, params) {
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

// Objet dans le namespace par défaut (absent, vide ou `default`) : genres DeploymentLike et Service.
fn use_namespace(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    if !(deployment_like(obj) || kind_of_group(obj, ["Service"], "")) { return []; }
    let ns = text(field(field(obj, "metadata"), "namespace"));
    if ns != "" && ns != "default" { return []; }
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    [render_lint::finding(
        "use-namespace",
        "error",
        obj,
        "metadata.namespace",
        `${kind} "${name}" : déployé dans le namespace par défaut ; le placer dans un namespace dédié (metadata.namespace)`
    )]
}

// Genres de workload qui peuvent être la cible d'un HorizontalPodAutoscaler, par groupe d'API.
// Mesuré sur kube-linter v0.8.3 : Deployment, StatefulSet, DaemonSet, ReplicaSet (apps) ; Job,
// CronJob (batch). Un Pod et un ReplicationController ne sont jamais résolus comme cible.
private fn scalable_kinds(group) {
    if group == "apps" {
        ["Deployment", "StatefulSet", "DaemonSet", "ReplicaSet"]
    } else if group == "batch" {
        ["Job", "CronJob"]
    } else {
        []
    }
}

// Vrai quand un objet de `objs` est la cible `api`/`kind`/`name` d'une référence de HPA. La
// résolution compare l'apiVersion exacte (groupe ET version, mesuré : une cible apps/v1beta2 ne
// résout pas une référence apps/v1) et le genre dans le groupe ; le namespace n'entre pas en jeu
// (mesuré : une cible d'un autre namespace est résolue). Une apiVersion sans groupe ne résout rien.
private fn target_exists(idx, api, kind, name) {
    import "render_lint" as render_lint;
    let group = render_lint::api_group_of(api);
    if type_of(group) != "string" || !(kind in scalable_kinds(group)) { return false; }
    idx.workload_identities.has(`${api}|${kind}|${name}`)
}

// HorizontalPodAutoscaler dont la cible `spec.scaleTargetRef` n'existe pas dans la liste d'objets.
// Seule vérification de ce module qui lit `objs` : la résolution se fait entre objets.
fn dangling_horizontalpodautoscaler(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    if !kind_of_group(obj, ["HorizontalPodAutoscaler"], "autoscaling") { return []; }
    let spec = field(obj, "spec");
    let target = field(spec, "scaleTargetRef");
    if type_of(target) != "map" { return []; }
    let api = text(field(target, "apiVersion"));
    let kind = text(field(target, "kind"));
    let tname = text(field(target, "name"));
    if target_exists(idx, api, kind, tname) { return []; }
    let name = text(field(field(obj, "metadata"), "name"));
    [render_lint::finding(
        "dangling-horizontalpodautoscaler",
        "error",
        obj,
        "spec.scaleTargetRef",
        `HorizontalPodAutoscaler "${name}" : la cible ${kind} "${tname}" (${api}) est absente de la liste d'objets ; créer la cible ou corriger spec.scaleTargetRef`
    )]
}

// HorizontalPodAutoscaler dont `spec.minReplicas` est inférieur à `minReplicas` (défaut 3). Absent
// ou nul : 1, valeur par défaut de Kubernetes. Une valeur qui n'est pas un entier (chaîne, décimal)
// est une forme inattendue : pas de constat. Tous les groupes autoscaling sont concernés.
fn hpa_minimum_three_replicas(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    if !kind_of_group(obj, ["HorizontalPodAutoscaler"], "autoscaling") { return []; }
    let spec = field(obj, "spec");
    if type_of(spec) != "map" { return []; }
    let raw = field(spec, "minReplicas");
    let minimum = if raw == () { 1 } else { raw };
    if type_of(minimum) != "i64" { return []; }
    let want = int_param(params, "minReplicas", 3);
    if minimum >= want { return []; }
    let name = text(field(field(obj, "metadata"), "name"));
    [render_lint::finding(
        "hpa-minimum-three-replicas",
        "error",
        obj,
        "spec.minReplicas",
        `HorizontalPodAutoscaler "${name}" : minReplicas ${minimum}, minimum ${want} ; porter spec.minReplicas à au moins ${want}`
    )]
}

// PodDisruptionBudget évaluée par kube-linter : seule l'apiVersion policy/v1 est lue ; une PDB
// policy/v1beta1 est silencieuse, même invalide (mesuré sur v0.8.3).
private fn is_pdb(obj) {
    text(field(obj, "apiVersion")) == "policy/v1" && kind_of_group(obj, ["PodDisruptionBudget"], "policy")
}

// Nom et namespace d'un objet (chaîne vide si absents).
private fn obj_name(obj) {
    text(field(field(obj, "metadata"), "name"))
}

private fn obj_namespace(obj) {
    text(field(field(obj, "metadata"), "namespace"))
}

// Workloads qu'une PDB peut protéger (mesuré) : Deployment, StatefulSet, ReplicaSet et DaemonSet
// (apps, toute version), Job et CronJob (batch). Un ReplicationController et un Pod ne sont pas liés.
private fn pdb_related(obj) {
    kind_of_group(obj, ["Deployment", "StatefulSet", "ReplicaSet", "DaemonSet"], "apps")
        || kind_of_group(obj, ["Job", "CronJob"], "batch")
}

// Réplicas comptés pour une PDB : `spec.replicas`, 1 quand il est absent (mesuré pour Deployment,
// StatefulSet, ReplicaSet). Les genres sans champ replicas (DaemonSet, Job, CronJob) comptent 0.
private fn pdb_replicas(dep) {
    if kind_of_group(dep, ["Deployment", "StatefulSet", "ReplicaSet"], "apps") {
        let r = field(field(dep, "spec"), "replicas");
        if r == () { 1 } else { r }
    } else {
        0
    }
}

// Vrai quand le sélecteur de la PDB (matchLabels et matchExpressions) s'applique aux labels `dl`
// du sélecteur du workload. Mesuré : la PDB {app: a} s'applique à un workload de sélecteur
// {app: a, tier: "yy"} (sous-ensemble), pas l'inverse ; un sélecteur vide s'applique à tout.
private fn pdb_selects(match_labels, exprs, dl) {
    if type_of(match_labels) == "map" {
        for k in match_labels.keys() {
            if !(k in dl) || dl[k] != match_labels[k] { return false; }
        }
    }
    if type_of(exprs) == "array" {
        for e in exprs {
            if !expression_matches(e, dl) { return false; }
        }
    }
    true
}

// Chaîne de pourcentage `N%` (chiffres puis %), vraie quand la valeur en est une.
private fn is_percent(v) {
    type_of(v) == "string" && regex_is_match("^[0-9]+%$", v)
}

// maxUnavailable à 0 bloque toute éviction. Entier ou décimal nul, ou pourcentage nul ("0%") ;
// une chaîne qui n'est pas un pourcentage (y compris un entier écrit entre guillemets) est une
// valeur invalide. Absent ou nul : pas de constat.
fn pdb_max_unavailable(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    if !is_pdb(obj) { return []; }
    let spec = field(obj, "spec");
    if type_of(spec) != "map" { return []; }
    let v = field(spec, "maxUnavailable");
    let zero = false;
    let invalid = false;
    if type_of(v) == "i64" {
        zero = v == 0;
    } else if type_of(v) == "f64" {
        zero = v == 0.0;
    } else if type_of(v) == "string" {
        if is_percent(v) {
            zero = regex_is_match("^0+%$", v);
        } else {
            invalid = true;
        }
    } else {
        return [];
    }
    let name = obj_name(obj);
    if zero {
        return [render_lint::finding(
            "pdb-max-unavailable",
            "error",
            obj,
            "spec.maxUnavailable",
            `PodDisruptionBudget "${name}" : maxUnavailable ${v} bloque toute éviction de pods ; définir spec.maxUnavailable à une valeur supérieure à 0`
        )];
    }
    if invalid {
        return [render_lint::finding(
            "pdb-max-unavailable",
            "error",
            obj,
            "spec.maxUnavailable",
            `PodDisruptionBudget "${name}" : maxUnavailable "${v}" n'est ni un entier ni un pourcentage ; utiliser un entier sans guillemets ou un pourcentage supérieur à 0`
        )];
    }
    []
}

// minAvailable qui ne laisse aucune éviction possible : un nombre de réplicas des workloads liés
// (même namespace, sélecteur couvert) inférieur ou égal à minAvailable. Entier, entier écrit en
// chaîne et décimal entier sont comptés comme entiers ; `N%` est arrondi au supérieur sur les
// réplicas de chaque workload ; `100%` est signalé une fois, sans workload. Chaîne invalide ou
// selector absent (nul compris) : constat dédié, une fois par PDB.
fn pdb_min_available(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    if !is_pdb(obj) { return []; }
    let spec = field(obj, "spec");
    if type_of(spec) != "map" { return []; }
    let v = field(spec, "minAvailable");
    let name = obj_name(obj);
    let count = -1;
    let percent = -1;
    if type_of(v) == "string" {
        if is_percent(v) {
            let p = parse_int(v.sub_string(0, v.len() - 1));
            if p == 100 {
                return [render_lint::finding(
                    "pdb-min-available",
                    "error",
                    obj,
                    "spec.minAvailable",
                    `PodDisruptionBudget "${name}" : minAvailable 100% bloque toute éviction ; définir une valeur strictement inférieure à 100%`
                )];
            }
            percent = p;
        } else if regex_is_match("^[0-9]+$", v) {
            count = parse_int(v);
        } else {
            return [render_lint::finding(
                "pdb-min-available",
                "error",
                obj,
                "spec.minAvailable",
                `PodDisruptionBudget "${name}" : minAvailable "${v}" n'est ni un entier ni un pourcentage ; utiliser un entier sans guillemets ou un pourcentage`
            )];
        }
    } else if type_of(v) == "i64" {
        count = v;
    } else if type_of(v) == "f64" && v == v.floor() {
        count = v.to_int();
    } else {
        return [];
    }
    let selector = field(spec, "selector");
    if selector == () {
        return [render_lint::finding(
            "pdb-min-available",
            "error",
            obj,
            "spec.selector",
            `PodDisruptionBudget "${name}" : spec.selector absent ; définir spec.selector pour désigner les workloads protégés`
        )];
    }
    if type_of(selector) != "map" { return []; }
    let match_labels = field(selector, "matchLabels");
    let exprs = field(selector, "matchExpressions");
    if malformed(match_labels) || malformed_list(exprs) { return []; }
    let ns = obj_namespace(obj);
    let out = [];
    for i in 0..idx.workload_pdb_targets.count(ns) {
        let dep = idx.workload_pdb_targets.at(ns, i);
        let dlabels = field(field(dep, "spec"), "selector");
        let dl = field(dlabels, "matchLabels");
        let labels = if type_of(dl) == "map" { dl } else { #{} };
        if !pdb_selects(match_labels, exprs, labels) { continue; }
        let replicas = pdb_replicas(dep);
        if type_of(replicas) != "i64" { continue; }
        let minimum = if percent >= 0 { (replicas * percent + 99) / 100 } else { count };
        if replicas > minimum { continue; }
        let dkind = text(field(dep, "kind"));
        let dname = obj_name(dep);
        out.push(render_lint::finding(
            "pdb-min-available",
            "error",
            obj,
            "spec.minAvailable",
            `PodDisruptionBudget "${name}" : ${dkind} "${dname}" compte ${replicas} réplica(s), minAvailable ${minimum} bloque toute éviction ; définir minAvailable strictement inférieur à ${replicas}`
        ));
    }
    out
}

// PodDisruptionBudget sans `unhealthyPodEvictionPolicy` explicite (absent ou nul). Toute valeur
// explicite est acceptée.
fn pdb_unhealthy_pod_eviction_policy(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    if !is_pdb(obj) { return []; }
    let spec = field(obj, "spec");
    if type_of(spec) != "map" { return []; }
    if field(spec, "unhealthyPodEvictionPolicy") != () { return []; }
    let name = obj_name(obj);
    [render_lint::finding(
        "pdb-unhealthy-pod-eviction-policy",
        "error",
        obj,
        "spec.unhealthyPodEvictionPolicy",
        `PodDisruptionBudget "${name}" : unhealthyPodEvictionPolicy non défini ; le fixer explicitement, par exemple à AlwaysAllow`
    )]
}

// Annotation de metadata.annotations dont la clé répond au motif `key` (recherche, non ancrée) et
// dont la valeur répond au motif `value` (non ancrée) : le couple doit correspondre (mesuré). Seules
// les métadonnées de l'objet comptent, pas le gabarit de pod.
// Défaut : annotation `email` à une adresse, comme la vérification de kube-linter.
fn required_annotation_email(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    if !deployment_like(obj) { return []; }
    let key = string_param(params, "key", "email");
    let value = string_param(params, "value", "[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\\.[a-zA-Z0-9-.]+");
    let annotations = field(field(obj, "metadata"), "annotations");
    if malformed(annotations) { return []; }
    if type_of(annotations) == "map" {
        for k in annotations.keys() {
            if regex_is_match(key, k) && regex_is_match(value, text(annotations[k])) { return []; }
        }
    }
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    [render_lint::finding(
        "required-annotation-email",
        "error",
        obj,
        "metadata.annotations",
        `${kind} "${name}" : aucune annotation "${key}" avec une valeur valide (${value}) ; ajouter metadata.annotations.${key} avec l'adresse e-mail du propriétaire`
    )]
}

// Label de metadata.labels dont la clé répond au motif `key` (recherche, non ancrée, sensible à la
// casse) ; la valeur est libre. Seules les métadonnées de l'objet comptent, pas le gabarit de pod.
// Défaut : label `owner`, comme la vérification de kube-linter.
fn required_label_owner(obj, idx, ctx, params) {
    import "render_lint" as render_lint;
    if !deployment_like(obj) { return []; }
    let key = string_param(params, "key", "owner");
    let labels = field(field(obj, "metadata"), "labels");
    if malformed(labels) { return []; }
    if type_of(labels) == "map" {
        for k in labels.keys() {
            if regex_is_match(key, k) { return []; }
        }
    }
    let kind = text(field(obj, "kind"));
    let name = text(field(field(obj, "metadata"), "name"));
    [render_lint::finding(
        "required-label-owner",
        "error",
        obj,
        "metadata.labels",
        `${kind} "${name}" : aucun label "${key}" ; ajouter metadata.labels.${key} avec le nom du propriétaire`
    )]
}

// Entrées d'index de ce thème, construites une fois par `render_lint::index` :
//   workload_identities  : `apiVersion|kind|nom` -> vrai, pour chaque objet (résolution de la cible
//                          d'un HorizontalPodAutoscaler) ;
//   workload_pdb_targets : namespace -> workload qu'une PodDisruptionBudget peut protéger.
fn index(objs) {
    let identities = multimap();
    let pdb_targets = multimap();
    for o in objs {
        identities.put(`${text(field(o, "apiVersion"))}|${text(field(o, "kind"))}|${obj_name(o)}`, true);
        if pdb_related(o) { pdb_targets.put(obj_namespace(o), o); }
    }
    #{ workload_identities: identities, workload_pdb_targets: pdb_targets }
}
