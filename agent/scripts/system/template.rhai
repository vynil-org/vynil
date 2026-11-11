fn run(instance, context) {
    let ctx = import_template("install_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    if host::is_dir(`${context.package_dir}/crds`) {
        ctx = import_run("template_crds", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
    if host::is_dir(`${context.package_dir}/systems`) {
        ctx = import_run("template_systems", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
}
