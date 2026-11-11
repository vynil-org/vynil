import "scan_harbor" as harbor;
import "scan_gitlab" as gitlab;
import "secret_dockerconfigjson" as secret;

fn maturity_filter(str, maturity) {
    if maturity == "alpha" {true}
    else if maturity == "beta" {!str.contains("alpha")}
    else {!str.contains("alpha") && !str.contains("beta")}
}

fn security_filter(v) {
    if v.len() == 0 { return false; }
    let c = v.sub_string(0, 1);
    if c != "v" && (c < "0" || c > "9") { return false; }
    try { semver::from(v); return true } catch(e) {
        host::log_warn(`skipping non-semver tag ${v}`);
    }
    false
}

fn scan_image_with_maturity(image, reg, all_tags, maturity) {
    let tags = all_tags
        .filter(|str| maturity_filter(str, maturity))
        .filter(|v| security_filter(v))
        .map(|v| semver::from(v));
    if tags.len() < 1 { return []; }
    tags.sort(|a, b| if a == b { 0 } else if a < b { 1 } else { -1 });
    let tags = tags.map(|v| v.to_string());

    let iter_tags = tags.shift();
    let last_min_version = ();
    let found = [];

    while iter_tags != () && type_of(iter_tags) == "string" && iter_tags != "" {
        let current = iter_tags;
        iter_tags = ();
        let annotations = reg.get_manifest(image.repository, current).annotations;
        if annotations.keys().contains("fr.solidite.vynil.metadata") && annotations.keys().contains("fr.solidite.vynil.requirements") {
            let cur = #{
                registry: image.registry,
                image: image.repository,
                tag: current,
                metadata: core::json_decode(annotations["fr.solidite.vynil.metadata"]),
                requirements: core::json_decode(annotations["fr.solidite.vynil.requirements"]),
            };
            if annotations.keys().contains("fr.solidite.vynil.options") {
                cur["options"] = core::json_decode(annotations["fr.solidite.vynil.options"]);
            }
            if annotations.keys().contains("fr.solidite.vynil.recommandations") {
                cur["recommandations"] = core::json_decode(annotations["fr.solidite.vynil.recommandations"]);
            }
            if annotations.keys().contains("fr.solidite.vynil.value_script") {
                cur["value_script"] = annotations["fr.solidite.vynil.value_script"];
            }
            if (type_of(cur["requirements"]) == "array" && cur["requirements"].len() > 0) {
                if (cur["requirements"].some(|i| i.vynil_version != () && security_filter(i.vynil_version) && semver::from(i.vynil_version)>semver::from(vynil_version()))) {
                    if tags.len() >= 1 {iter_tags = tags.shift();}
                    print(`Ignoring ${image.repository} as it requiere a too new version, ${iter_tags}, ${current}, ${tags.len}`);
                } else if (last_min_version != () && cur["requirements"].some(|i| i.minimum_previous_version != () && security_filter(i.minimum_previous_version) && semver::from(i.minimum_previous_version)>=semver::from(last_min_version))) {
                    if tags.len() >= 1 {iter_tags = tags.shift();}
                } else {
                    if cur["requirements"].some(|i| i.minimum_previous_version != () && security_filter(i.minimum_previous_version)) {
                        last_min_version = cur["requirements"].find(|i| i.minimum_previous_version != () && security_filter(i.minimum_previous_version)).minimum_previous_version;
                        if tags.len() >= 1 {iter_tags = tags.shift();}
                    } else {
                        last_min_version = ();
                    }
                    found += cur;
                }
            } else {
                found += cur;
            }
        }
    }
    found
}

fn union_packages(sets) {
    let seen = #{};
    let result = [];
    for pkg in sets.reduce(|acc, x| acc + x, []) {
        if !seen.contains_key(pkg.tag) {
            seen[pkg.tag] = true;
            result.push(pkg);
        }
    }
    result
}

fn compute_waypoints_from_packages(packages, maturity) {
    let filtered = packages
        .filter(|p| maturity_filter(p.tag, maturity))
        .filter(|p| security_filter(p.tag));
    let sorted_sv = filtered.map(|p| #{sv: semver::from(p.tag), pkg: p});
    sorted_sv.sort(|a, b| if a.sv == b.sv { 0 } else if a.sv < b.sv { 1 } else { -1 });
    let sorted = sorted_sv.map(|v| v.pkg);

    let iter = sorted.shift();
    let last_min_version = ();
    let found = [];
    while iter != () && type_of(iter) == "map" {
        let cur = iter; iter = ();
        if type_of(cur.requirements) == "array" && cur.requirements.len() > 0 {
            if cur.requirements.some(|i|
                i.vynil_version != () &&
                security_filter(i.vynil_version) &&
                semver::from(i.vynil_version) > semver::from(vynil_version())
            ) {
                if sorted.len() >= 1 { iter = sorted.shift(); }
            } else if last_min_version != () && cur.requirements.some(|i|
                i.minimum_previous_version != () &&
                security_filter(i.minimum_previous_version) &&
                semver::from(i.minimum_previous_version) >= semver::from(last_min_version)
            ) {
                if sorted.len() >= 1 { iter = sorted.shift(); }
            } else {
                if cur.requirements.some(|i|
                    i.minimum_previous_version != () &&
                    security_filter(i.minimum_previous_version)
                ) {
                    last_min_version = cur.requirements
                        .find(|i| i.minimum_previous_version != () && security_filter(i.minimum_previous_version))
                        .minimum_previous_version;
                    if sorted.len() >= 1 { iter = sorted.shift(); }
                } else {
                    last_min_version = ();
                }
                found += cur;
            }
        } else {
            found += cur;
        }
    }
    found
}

let scan_filter = if "filter" in args.keys() && args.filter != () { args.filter } else { () };
let file_scan   = "file_scan" in args.keys() && args.file_scan == true;

let spec = box.spec;
let use_auth = "pull_secret" in spec.keys() && spec.pull_secret!="" && spec.pull_secret!=();
try {
    let found = [];
    if "http" in spec.source.keys() {
        let http_auth_type = if "http_auth_type" in args.keys() { args.http_auth_type } else { "" };
        let http_credential = if "http_credential" in args.keys() { args.http_credential } else { "" };
        let base_url = spec.source.http.url;
        if base_url.ends_with("/") { base_url = base_url.sub_string(0, base_url.len()-1); }

        let index = http_get_yaml(`${base_url}/index.yaml`, http_auth_type, http_credential);
        for entry in index.packages {
            if scan_filter != () {
                let f = scan_filter.split("/");
                if entry.category != f[0] { continue; }
                if f.len() > 1 && entry.name != f[1] { continue; }
            }
            let pkgs = http_get_yaml(`${base_url}/${entry.file}`, http_auth_type, http_credential);
            found += compute_waypoints_from_packages(pkgs, spec.maturity);
        }
    } else if "s3" in spec.source.keys() {
        let access_key = if "s3_access_key" in args.keys() { args.s3_access_key } else { "" };
        let secret_key = if "s3_secret_key" in args.keys() { args.s3_secret_key } else { "" };
        let s3 = spec.source.s3;
        let bucket   = s3.bucket;
        let region   = s3.region;
        let prefix   = s3.prefix ?? "";
        let endpoint = s3.endpoint ?? "";

        let index = s3_get_yaml(bucket, region, prefix, endpoint, access_key, secret_key, "index.yaml");
        for entry in index.packages {
            if scan_filter != () {
                let f = scan_filter.split("/");
                if entry.category != f[0] { continue; }
                if f.len() > 1 && entry.name != f[1] { continue; }
            }
            let pkgs = s3_get_yaml(bucket, region, prefix, endpoint, access_key, secret_key, entry.file);
            found += compute_waypoints_from_packages(pkgs, spec.maturity);
        }
    } else {
        let images_list = if "harbor" in spec.source.keys() {
                harbor::list_repository(spec, args.namespace)
            } else if "gitlab" in spec.source.keys() {
                gitlab::list_repository(spec, args.namespace)
            } else if "script" in spec.source.keys() {
                eval(spec.source.script)
            } else {
                spec.source.list
            };
        let list = images_list.map(|image| {
            let tmp = image.split("/");
            let reg = tmp.shift();
            #{registry: reg, repository: tmp.reduce(|sum| if sum=="" {this} else {`${sum}/${this}`}, "")}
        });

        let list = if scan_filter == () {
            list
        } else {
            let filter_parts = scan_filter.split("/");
            let filter_cat   = filter_parts[0];
            let filter_name  = if filter_parts.len() > 1 { filter_parts[1] } else { () };
            let status_pkgs = [];
            try { status_pkgs = box.status.packages; } catch(e) {}
            let matched = status_pkgs
                .filter(|p|
                    p.metadata.category == filter_cat &&
                    (filter_name == () || p.metadata.name == filter_name)
                )
                .map(|p| #{registry: p.registry, repository: p.image});
            let seen = #{}; let deduped = [];
            for m in matched {
                let k = `${m.registry}/${m.repository}`;
                if k in seen {} else { seen[k] = true; deduped.push(m); }
            }
            if deduped.len() > 0 {
                deduped
            } else {
                host::log_warn(`partial scan: filter '${scan_filter}' matched no known package — run a full scan first`);
                []
            }
        };

        for image in list {
            let auth = if !use_auth {
                #{user: "", pass: ""}
            } else if file_scan {
                get_auth_from_file(spec.pull_secret, image.registry)
            } else {
                secret::get_auth_from(spec.pull_secret, args.namespace, image.registry)
            };
            host::log_info(`Scanning ${image.registry}/${image.repository}`);
            let reg = new_registry(image.registry, auth.user, auth.pass);
            let got_tags = reg.list_tags(image.repository);
            let image_found = if file_scan {
                union_packages([
                    scan_image_with_maturity(image, reg, got_tags, "stable"),
                    scan_image_with_maturity(image, reg, got_tags, "beta"),
                    scan_image_with_maturity(image, reg, got_tags, "alpha"),
                ])
            } else {
                scan_image_with_maturity(image, reg, got_tags, spec.maturity)
            };
            found += image_found;
        }
    }

    if scan_filter != () {
        let tmp = box.set_status_packages_merge(scan_filter, found);
    } else {
        let tmp = box.set_status_updated(found);
    }
} catch(e) {
    if type_of(e) == "string" {
        host::log_warn(e);
        let tmp = box.set_status_failed(e);
    } else {
        host::log_warn(core::json_encode(e));
        let tmp = box.set_status_failed(core::json_encode(e));
    }
    throw;
};
