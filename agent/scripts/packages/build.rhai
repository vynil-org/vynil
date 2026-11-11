import "copy_dir" as copy;
import "package_yaml" as pck;

fn security_filter(v) {
    if v.len() == 0 { return false; }
    let c = v.sub_string(0, 1);
    if c != "v" && (c < "0" || c > "9") { return false; }
    try { semver::from(v); return true } catch(e) {
        host::log_warn(`skipping non-semver tag ${v}`);
    }
    false
}

fn improve_package(src, dest) {
    if ! host::is_file(src) {
        throw src+" doesnt exist";
    }
    let yaml = core::yaml_decode(host::file_read(src));
    yaml = pck::validate(yaml);
    host::file_write(dest, core::yaml_encode(yaml));
}
fn build(args) {
    host::create_dir(args.temp);
    improve_package(args.source+"/package.yaml", args.temp+"/package.yaml");
    // use Rust package.yaml loading for final validation
    let valid = read_package_yaml(args.temp+"/package.yaml");
    valid.validate_options();
    // Duplicate supported directory
    for item in host::read_dir(args.source) {
        let base = core::basename(item);
        if base in ["systems", "crds"] && host::is_dir(item) && valid.metadata.type == "system" {
            copy::dir_exts(item, args.temp+"/"+base, [".hbs", ".yaml", ".yml"]);
        } else if base == "crds" && host::is_dir(item) && valid.metadata.type == "service" {
            copy::dir_exts(item, args.temp+"/"+base, [".hbs", ".yaml", ".yml"]);
        } else if base in ["vitals", "scalables", "others", "befores", "posts"] && host::is_dir(item) && valid.metadata.type in ["tenant", "service"] {
            copy::dir_exts(item, args.temp+"/"+base, [".hbs", ".yaml", ".yml"]);
        } else if base == "pods" && host::is_dir(item) && valid.metadata.type in ["tenant", "service"] {
            copy::dir_exts(item, args.temp+"/"+base, [".hbs"]);
        } else if base == "handlebars" && host::is_dir(item) {
            for sub in host::read_dir(item) {
                let subbase = core::basename(sub);
                if subbase == "partials"  && host::is_dir(sub) {
                    copy::dir_exts(sub, `${args.temp}/${base}/${subbase}`, [".hbs"]);
                } else if subbase == "helpers"  && host::is_dir(sub) {
                    copy::dir_exts(sub, `${args.temp}/${base}/${subbase}`, [".rhai"]);
                }
            }
        } else if base == "scripts" && host::is_dir(item) {
            copy::dir_exts(item, args.temp+"/"+base, [".rhai", ".sh"], true);
        } else if base == "tofu" && host::is_dir(item) {
            copy::dir_exts(item, args.temp+"/"+base, [".tf", ".tpl", ".tftpl"], true);
        } else if base == "files" && host::is_dir(item) {
            copy::dir_all(item, args.temp+"/"+base, true);
        }
    }
    valid
}

fn run(args) {
    let valid = build(args);
    let reg = new_registry(args.registry, args.username, args.password);
    let version = semver::from("0.1.0");
    if args.tag != "" {
        version = semver::from(args.tag);
    } else {
        try {
            let tags = reg.list_tags(args.repository)
                .filter(|v| security_filter(v))
                .map(|v| semver::from(v));
            tags.sort(|a,b| if a==b {0} else if a<b {1} else {-1});
            if tags.len() > 0 {
                version = tags[0];
            }
        } catch (e) {
            switch type_of(e) {
                "string" => host::log_warn(e),
                _ => host::log_warn(core::json_encode(e))
            }
        }
        switch args.build {
            "major" => version.inc_major(),
            "minor" => version.inc_minor(),
            "patch" => version.inc_patch(),
            "beta"  => version.inc_beta(),
            "alpha" => version.inc_alpha(),
        }
    }
    let annotations = #{
        "fr.solidite.vynil.metadata": core::json_encode(valid.metadata),
        "fr.solidite.vynil.requirements": core::json_encode(valid.requirements),
    };
    if valid.options != () {
        annotations["fr.solidite.vynil.options"] = core::json_encode(valid.options);
    }
    if valid.recommandations != () {
        annotations["fr.solidite.vynil.recommandations"] = core::json_encode(valid.recommandations);
    }
    if valid.value_script != () && valid.value_script != "" {
        annotations["fr.solidite.vynil.value_script"] = core::json_encode(valid.value_script);
    }
    let str_ver = version.to_string();
    let digest = reg.push_image(args.temp, args.repository, str_ver, annotations);
    if args.signing_key != () && args.signing_key != "" {
        reg.sign_image(args.repository, str_ver, digest, args.signing_key);
    }
    print(`${args.repository} version ${str_ver} uploaded`);
}
