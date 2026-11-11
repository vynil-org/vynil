import "yaml_docs" as yd;
fn run(instance, context) {
    let ctx = import_template("install_crds_pre", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    let hbs = new_hbs();
    hbs.register_partial_dir(context.config_dir);
    hbs.register_partial_dir(`${context.package_dir}/handlebars/partials`);
    hbs.register_helper_dir(`${context.package_dir}/handlebars/helpers`);
    for file in host::read_dir(`${context.package_dir}/crds`) {
        let crds = if file.ends_with(".yaml") || file.ends_with(".yml") {
            yd::decode(host::file_read(file))
        } else if file.ends_with(".yaml.hbs") || file.ends_with(".yml.hbs") {
            yd::decode(hbs.render_named(file, host::file_read(file), context))
        } else {[]};
        for crd in crds {
            if crd.kind == "CustomResourceDefinition" {
                print("---");
                print(core::yaml_encode(crd));
            } else {
                print("---");
                host::log_warn("# Ignoring an item as it is not a CRD");
                host::log_debug(core::yaml_encode(crd));
            }
        }
    }
    ctx = import_template("install_crds_post", instance, context);
    if type_of(ctx) == "map" {
        context = ctx;
    }
    context
}
