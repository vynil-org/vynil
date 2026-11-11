import "wait" as wait;
fn run(instance, context) {
    let ctx = import_run("delete_vitals_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    // TODO : trigger a backup before cleaning
    /*if instance.status != () && instance.status.vitals != () && context.namespace.use_backup {
        let hbs = new_hbs();
        hbs.register_partial_dir(context.template_dir);
        if host::is_dir(`${context.package_dir}/pods`) {
            hbs.register_partial_dir(`${context.package_dir}/pods`);
        }
        let all = core::yaml_decode_multi(hbs.render_from("{{> manualbackup.yaml }}", context));
        for obj in all {
            dir::install_allowed_obj(obj, context.tenant.namespaces, instance.metadata.namespace);
        }
    }*/
    // TODO : wait for the backup-job to finish
    if instance.status != () && instance.status.vitals != () {
        let deleted = [];
        for old in instance.status.vitals {
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
    ctx = import_run("delete_vitals_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
