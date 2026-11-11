import "tofu_gen" as tfg;
fn run(instance, context) {
    let ctx = import_run("delete_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    if host::is_dir(`${context.package_dir}/posts`) {
        ctx = import_run("delete_posts", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
    if host::is_dir(`${context.package_dir}/scalables`) {
        ctx = import_run("delete_scalables", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
    if tfg::has_tofu_files(`${context.package_dir}/tofu`) {
        context = import_run("delete_tofu", instance, context);
    }
    if host::is_dir(`${context.package_dir}/others`) {
        ctx = import_run("delete_others", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
    if host::is_dir(`${context.package_dir}/vitals`) {
        import_run("delete_backup", instance, context);
        ctx = import_run("delete_vitals", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
    if host::is_dir(`${context.package_dir}/befores`) {
        ctx = import_run("delete_befores", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
    if host::is_dir(`${context.package_dir}/crds`) {
        ctx = import_run("delete_crds", instance, context);
        if type_of(ctx) == "map" {
            context = ctx;
        }
    }
    import_run("delete_post", instance, context);
}
