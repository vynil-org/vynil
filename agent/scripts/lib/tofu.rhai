fn to_tf(v) {
    let str = core::json_encode_escape(v);
    host::shell_output(`echo 'jsondecode(${str})'|tofu console`)
}

fn get_tf_type(item, optional) {
    switch item.type {
        "string"  => {if optional {"optional(string)"} else {"string"}},
        "integer" => {if optional {"optional(number)"} else {"number"}},
        "number"  => {if optional {"optional(number)"} else {"number"}},
        "boolean" => {if optional {"optional(bool)"} else {"bool"}},
        "object"  => {
            let ret = "";
            for k in item.properties.keys() {
                if ! ret.is_empty() {
                    ret += ", ";
                }
                let t = get_tf_type(item.properties[k], true);
                ret += `${k} = ${t}`
            }
            if ret.is_empty() && optional {
                "optional(map(any))"
            } else if ret.is_empty() {
                "map(any)"
            } else if optional {
                `optional(object({${ret}}))`
            } else {
                `object({${ret}})`
            }
        },
        "array"   => {
            if item.items == () {
                if optional {"optional(list(any))"} else {"list(any)"}
            } else {
                let t = get_tf_type(item.items, false);
                if optional {`optional(list(${t}))`} else {`list(${t})`}
            }
        },
    }
}

fn run_init(path) {
    let cfg = `provider_installation {
  filesystem_mirror {
    path    = "/nonexistent/.terraform.d/plugins"
    include = ["*/*"]
  }
  direct {}
}`;
    host::file_write(`${path}/.tofurc`, cfg);
    let rc = host::shell_run(`cd ${path};TF_CLI_CONFIG_FILE=.tofurc tofu init`);
    if rc != 0 {
        throw "tofu init failed";
    }
}
fn run_plan(path) {
    let rc = host::shell_run(`cd ${path};tofu plan -input=false -out=tf.plan -var-file=vynil.tfvars`);
    if rc != 0 {
        throw "tofu plan failed";
    }
}
fn run_apply(path) {
    let rc = host::shell_run(`cd ${path};tofu apply -input=false -auto-approve tf.plan`);
    if rc != 0 {
        throw "tofu apply failed";
    }
}
fn run_destroy(path) {
    let rc = host::shell_run(`cd ${path};tofu apply -destroy -input=false -auto-approve -var-file=vynil.tfvars`);
    if rc != 0 {
        throw "tofu destroy failed";
    }
}
fn get_output(path) {
    host::shell_output(`cd ${path};tofu output -json`)
}
