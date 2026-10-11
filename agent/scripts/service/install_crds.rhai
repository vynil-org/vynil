import "yaml_docs" as yd;
fn run(instance, context) {
    let ctx = import_run("install_crds_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    let hbs = new_hbs();
    hbs.register_partial_dir(context.config_dir);
    hbs.register_partial_dir(`${context.package_dir}/handlebars/partials`);
    hbs.register_helper_dir(`${context.package_dir}/handlebars/helpers`);
    let applied_crds = [];
    let api = k8s_resource("CustomResourceDefinition");
    try {
        for file in read_dir(`${context.package_dir}/crds`) {
            let crds = if file.ends_with(".yaml") || file.ends_with(".yml") {
                yd::decode(file_read(file))
            } else if file.ends_with(".yaml.hbs") || file.ends_with(".yml.hbs") {
                yd::decode(hbs.render_named(file, file_read(file), context))
            } else {[]};
            for crd in crds {
                if crd.kind == "CustomResourceDefinition" {
                    if context.cluster.crds.contains(crd.metadata.name) {
                        // Setting the same resourceVersion so the apiServer is happy
                        // see: https://github.com/argoproj/argo-cd/issues/3657#issuecomment-722706739
                        crd.metadata.resourceVersion = api.get_meta(crd.metadata.name).metadata.resourceVersion;
                        api.replace(crd.metadata.name, crd);
                    } else {
                        api.create(crd);
                    }
                    applied_crds.push(crd.metadata.name);
                } else {
                    log_warn("Ignoring an item as it is not a CRD");
                    log_debug(yaml_encode(crd));
                }
            }
        }
        if instance.status != () && instance.status.crds != () {
            for old in instance.status.crds {
                if ! applied_crds.some(|newi| newi == old) {
                    let old_api = k8s_resource(old.split(".")[0]);
                    if old_api.exist() {
                        let cnt = old_api.list_meta().items.len();
                        if cnt > 0 {
                            log_warn(`Ignoring to delete ${old} as it still have ${cnt} children`);
                            applied_crds.push(old);
                        } else {
                            api.delete(old);
                        }
                    }
                }
            }
        }
        let tmp = instance.set_status_crds(applied_crds);
    } catch (e) {
        switch type_of(e) {
            "string" => instance.set_status_crd_failed(e),
            _ => instance.set_status_crd_failed(json_encode(e))
        }
        throw;
    }
    ctx = import_run("install_crds_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
