fn validate(yaml) {
    if ! ("apiVersion" in yaml.keys()) {
        throw "You should set your apiVersion";
    }
    if ! ("kind" in yaml.keys()) {
        throw "You should set your kind";
    }
    if ! ("metadata" in yaml.keys()) {
        throw "No metadata defined in package.yaml";
    }
    for key in ["name","category", "type"] {
        if ! (key in yaml.metadata.keys()) {
            throw `No metadata.${key} defined in package.yaml`;
        } else if type_of(yaml.metadata[key]) != "string" {
            throw `metadata.${key} should be a string`;
        }
    }
    if "options" in yaml.keys() && type_of(yaml.options) != "map" {
        throw "options should be a map.";
    }
    if ! ("requirements" in yaml.keys()) {
        yaml.requirements = []
    } else if type_of(yaml.requirements) != "array" {
        throw "requirements should be an array.";
    }
    yaml
}

fn run(args) {
    let yaml = core::yaml_decode(host::file_read(args.source+"/package.yaml"));
    validate(yaml);
    let valid = read_package_yaml(args.source+"/package.yaml");
    valid.validate_options();
}
