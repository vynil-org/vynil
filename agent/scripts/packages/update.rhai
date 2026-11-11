import "package_yaml" as pck;
fn run(args) {
    import_run("update_pre", args);

    // scan all the images from the package.yaml for new tags
    // if new tags are found, update the package.yaml accordingly
    let yaml = yaml_decode_ordered(host::file_read(args.source+"/package.yaml"));
    let app_version = if type_of(yaml["metadata"]) == "map" {yaml["metadata"]["app_version"]} else {()};
    let new_version = ();
    let changed = false;
    if type_of(yaml["images"]) == "map" {
        for k in yaml["images"].keys() {
            host::log_info(`Scanning ${yaml["images"][k]["registry"]}/${yaml["images"][k]["repository"]}`);
            let reg = new_registry(yaml["images"][k]["registry"], "", "");
            let tags = reg.list_tags(yaml["images"][k]["repository"])
                .filter(|v| try{let x = semver::from(v);true}catch(e){false})
                .map(|v| semver::from(v));
            if tags.len() < 1 {continue;} // No found tags, skip
            tags.sort(|a,b| if a==b {0} else if a<b {1} else {-1});
            let new_tag = tags[0];
            if type_of(yaml["images"][k]["tag"]) == "string" {
                try {
                    let old_tag = semver::from(yaml["images"][k]["tag"]);
                    if new_tag > old_tag {
                        yaml["images"][k]["tag"] = new_tag.to_string();
                        changed = true;
                    }
                } catch (e) {
                    switch type_of(e) {
                        "string" => host::log_warn(e),
                        _ => host::log_warn(core::json_encode(e))
                    }
                }
            } else if app_version != () {
                if new_version != () && new_version != new_tag {
                    let newt = new_tag.to_string();
                    let found = new_version.to_string();
                    throw `Tag found for ${yaml["images"][k]["registry"]}/${yaml["images"][k]["repository"]} (${newt}) doesn't match the tag found for other images (${found})`;
                } else if new_version == () {
                    new_version = new_tag;
                }
            }
        }
        if new_version != () {
            try {
                let old_v = semver::from(app_version);
                if new_version > old_v {
                    yaml["metadata"]["app_version"] = new_version.to_string();
                    host::file_write(args.source+"/package.yaml", yaml_encode_ordered(yaml));
                }
            } catch (e) {
                if changed {
                    yaml = pck::validate(yaml);
                    host::file_write(args.source+"/package.yaml", yaml_encode_ordered(yaml));
                }
            }
        } else if changed {
            yaml = pck::validate(yaml);
            host::file_write(args.source+"/package.yaml", yaml_encode_ordered(yaml));
        }
    }

    import_run("update_post", args);
}
