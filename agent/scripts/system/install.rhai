import "tofu_gen" as tfg;
fn run(instance, context) {
    let ctx = import_run("install_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    if host::is_dir(`${context.package_dir}/crds`) {
        host::log_info(`Installing CRDs from ${context.package_dir}/crds from ${instance.metadata.namespace} ${instance.metadata.name}`);
        ctx = import_run("install_crds", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
        update_k8s_crd_cache();
        instance = get_system_instance(instance.metadata.namespace, instance.metadata.name);
    }
    if tfg::has_tofu_files(`${context.package_dir}/tofu`) {
        host::log_info(`Running tofu apply from ${context.package_dir}/tofu from ${instance.metadata.namespace} ${instance.metadata.name}`);
        context = import_run("install_tofu", instance, context);
        instance = get_system_instance(instance.metadata.namespace, instance.metadata.name);
    }
    if host::is_dir(`${context.package_dir}/systems`) {
        host::log_info(`Installing systems from ${context.package_dir}/systems from ${instance.metadata.namespace} ${instance.metadata.name}`);
        ctx = import_run("install_systems", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
        instance = get_system_instance(instance.metadata.namespace, instance.metadata.name);
    }
    import_run("install_post", instance, context);
    instance.set_status_ready(context.instance.requested);
}
