fn run(instance, context) {
    let ctx = import_template("install_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    if host::is_dir(`${context.package_dir}/befores`) {
        ctx = import_run("template_befores", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
    if host::is_dir(`${context.package_dir}/vitals`) {
        ctx = import_run("template_vitals", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
    if host::is_dir(`${context.package_dir}/others`) {
        ctx = import_run("template_others", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
    if host::is_dir(`${context.package_dir}/scalables`) {
        ctx = import_run("template_scalables", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
    if host::is_dir(`${context.package_dir}/posts`) {
        ctx = import_run("template_posts", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
}
