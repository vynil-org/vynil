fn run(instance, context) {
    let ctx = import_run("delete_scalables_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    if instance.status != () && instance.status.scalables != () {
        let deleted = [];
        for old in instance.status.scalables {
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
    }
    ctx = import_run("delete_scalables_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
