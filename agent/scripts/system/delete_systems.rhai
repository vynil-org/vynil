// Do not delete these at first, and only in this order
fn get_last() { [
    "Service",
    "Deployment",
    "ServiceAccount",
    "ClusterRole",
    "ClusterRoleBinding",
    "Role",
    "RoleBinding",
    "Secret",
    "ConfigMap",
    "PersistentVolume",
    "PersistentVolumeClaim",
    "Namespace",
] }

fn run(instance, context) {
    let ctx = import_run("delete_systems_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    if instance.status != () && instance.status.systems != () {
        let deleted = [];
        for old in instance.status.systems.filter(|o| !get_last().contains(o.kind)) {
            let ns = if old.namespace != () {old.namespace} else {instance.metadata.namespace};
            let api = if old.api_version != () {
                k8s_resource(old.api_version, old.kind, ns)
            } else if old.apiVersion != () {
                k8s_resource(old.apiVersion, old.kind, ns)
            } else {
                k8s_resource(old.kind, ns)
            };
            if api.exist() {
                try {
                    let o = api.get_obj(old.name);
                    o.delete();
                    deleted.push(o);
                } catch (e) {
                    switch type_of(e) {
                        "string" => if ! e.contains("NotFound") { host::log_warn(e) },
                        _ => host::log_warn(core::json_encode(e))
                    }
                }
            }
        }
        for d in deleted {
            host::log_info(`Waiting for ${d.original_kind} ${d.metadata.name} to be deleted`);
            d.wait_deleted(60*5);
        }
        deleted = [];
        for k in get_last() {
            for old in instance.status.systems.filter(|o| o.kind==k) {
                let ns = if old.namespace != () {old.namespace} else {instance.metadata.namespace};
                let api = if old.api_version != () {
                    k8s_resource(old.api_version, old.kind, ns)
                } else if old.apiVersion != () {
                    k8s_resource(old.apiVersion, old.kind, ns)
                } else {
                    k8s_resource(old.kind, ns)
                };
                if api.exist() {
                    try {
                        let o = api.get_obj(old.name);
                        o.delete();
                        deleted.push(o);
                    } catch (e) {
                        switch type_of(e) {
                            "string" => if ! e.contains("NotFound") { host::log_warn(e) },
                            _ => host::log_warn(core::json_encode(e))
                        }
                    }
                }
            }
        }
        for d in deleted {
            host::log_info(`Waiting for ${d.original_kind} ${d.metadata.name} to be deleted`);
            d.wait_deleted(60*5);
        }
    }
    ctx = import_run("delete_systems_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
