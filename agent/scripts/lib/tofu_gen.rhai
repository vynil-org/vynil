import "tofu" as tf;
fn has_tofu_files(path) {
    if !host::is_dir(path) { return false; }
    let generated = ["00_vynil_vars.tf", "00_vynil_locals.tf", "providers.tf"];
    for f in host::read_dir(path) {
        let b = core::basename(f);
        if core::glob(b, "*.tf") && !generated.contains(b) { return true; }
    }
    false
}
fn gen_files(context) {
    gen_provider(`${context.package_dir}/tofu`);
    gen_locals(context);
    gen_vars(context.instance["package"].options, `${context.package_dir}/tofu`);
    gen_tfvars(context);
}

fn gen_vars(options, path) {
    let str = "";
    if options != () {
        for k in options.keys() {
            let t = tf::get_tf_type(options[k], false);
            let d = tf::to_tf(options[k]["default"]);
            str += `variable "${k}" {
  default = ${d}  type    = ${t}
}

`;
        }
    }
    host::file_write(`${path}/00_vynil_vars.tf`, str);
}

fn gen_tfvars(context) {
    let str = "";
    if context.values != () {
        for k in context.values.keys() {
            let v = tf::to_tf(context.values[k]);
            str+=`${k} = ${v}
`;
        }
    }
    host::file_write(`${context.package_dir}/tofu/vynil.tfvars`, str);
}

fn gen_locals(context) {
    let cluster = tf::to_tf(context.cluster);
    let instance = tf::to_tf(context.instance);
    let defaults = tf::to_tf(context.defaults);
    let extra = tf::to_tf(context.extra);
    let owner = tf::to_tf(vynil_owner());
    host::file_write(`${context.package_dir}/tofu/00_vynil_locals.tf`, `locals {
    vynil_cluster = ${cluster}
    vynil_instance = ${instance}
    vynil_defaults = ${defaults}
    vynil_extra = ${extra}
    vynil_owner = ${owner}
}`);
}

fn cached_version(namespace, name) {
    let base = `/nonexistent/.terraform.d/plugins/registry.opentofu.org/${namespace}/${name}`;
    if !host::is_dir(base) { return ""; }
    let entries = host::read_dir(base);
    if entries.len() == 0 { return ""; }
    core::basename(entries[0])
}
fn provider_constraint(namespace, name, fallback) {
    let ver = cached_version(namespace, name);
    if ver != "" { `= ${ver}` } else { fallback }
}
fn gen_provider(path) {
    let k8s_ver     = provider_constraint("hashicorp",   "kubernetes", "~> 2.34.0");
    let kubectl_ver = provider_constraint("gavinbunney", "kubectl",    "~> 1.16.0");
    // HCL ${file(...)} syntax stored in variables to avoid Rhai interpolation conflicts
    let sa_token = "${file(\"/run/secrets/kubernetes.io/serviceaccount/token\")}";
    let sa_cert  = "${file(\"/run/secrets/kubernetes.io/serviceaccount/ca.crt\")}";
    host::file_write(`${path}/providers.tf`, `terraform {
  required_providers {
    kubernetes = {
        source  = "hashicorp/kubernetes"
        version = "${k8s_ver}"
    }
    kubectl = {
        source  = "gavinbunney/kubectl"
        version = "${kubectl_ver}"
    }
  }
}
provider "kubernetes" {
    host                   = "https://kubernetes.default.svc"
    token                  = "${sa_token}"
    cluster_ca_certificate = "${sa_cert}"
}
provider "kubectl" {
    host                   = "https://kubernetes.default.svc"
    token                  = "${sa_token}"
    cluster_ca_certificate = "${sa_cert}"
    load_config_file       = false
}`);
}
