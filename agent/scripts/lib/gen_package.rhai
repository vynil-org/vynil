fn gen_yaml(path, data) {
    host::file_write(path, `---
${core::yaml_encode(data)}`);
}
fn gen_crd_yaml(path, data) {
    host::file_write(path, `# yamllint disable rule:line-length
---
${core::yaml_encode(data)}`);
}
fn gen_yaml_if(path, type, data) {
    host::file_write(path, `{{#if (ctx_have_crd this "${type}")}}
---
${core::yaml_encode(data)}
{{/if}}
`);
}

// Generate a kubernetes-valid random placeholder string (vXXXXXXXX).
// Use as Helm release name AND --namespace= value so every occurrence in the
// generated templates can be reliably replaced with {{instance.appslug}} or
// {{instance.namespace}} without false positives.
fn placeholder() {
    let result = "vnsplaceholder";
    try {
        let r = host::shell_output("cat /proc/sys/kernel/random/uuid 2>/dev/null");
        r.trim();                              // mute r en place, ne pas chaîner (retourne ())
        let parts = r.split("-");
        if parts.len() >= 1 && parts[0].len() >= 4 {
            result = `v${parts[0]}`;           // UUID déjà en minuscules
        }
    } catch(e) {}
    result
}

// Replace only the first occurrence of `from` in `s` with `to`.
// Uses split() to reconstruct: first_part + to + rest_joined_with_from.
fn replace_first(s, from, to) {
    if !s.contains(from) { return s; }
    let parts = s.split(from);
    if parts.len() <= 1 { return s; }
    let first = parts[0];
    let tail_parts = [];
    let i = 0;
    for p in parts { if i > 0 { tail_parts.push(p); } i += 1; }
    let tail = tail_parts.reduce(|acc, x| if acc == "" { x } else { `${acc}${from}${x}` }, "");
    `${first}${to}${tail}`
}

// Generic text-level replacement across all files of a generated sub-directory.
fn apply_replace_in_dir(dir_path, from, to) {
    if !host::is_dir(dir_path) { return; }
    for file in host::read_dir(dir_path) {
        if host::is_file(file) {
            let content = host::file_read(file);
            content.replace(from, to);
            host::file_write(file, content);
        }
    }
}
fn apply_replace_to_generated(path, from, to) {
    for dir in ["get_crds", "get_systems", "get_scalables", "get_others", "get_vitals"] {
        apply_replace_in_dir(`${path}/${dir}`, from, to);
    }
}

fn apply_namespace_to_generated(path, ns) {
    if ns == () || ns == "" { return; }
    apply_replace_to_generated(path, ns, "{{instance.namespace}}");
}

// Replace every remaining occurrence of the release-name placeholder with {{instance.appslug}}.
// Catches annotation values and other string fields not covered by structural replacements.
fn apply_name_to_generated(path, name) {
    if name == () || name == "" { return; }
    apply_replace_to_generated(path, name, "{{instance.appslug}}");
}

// Read the package name from package.yaml (used to build the dedup pattern).
fn read_pkg_name(path) {
    if !host::is_file(`${path}/package.yaml`) { return (); }
    let pkg_meta = core::yaml_decode(host::file_read(`${path}/package.yaml`));
    if type_of(pkg_meta["metadata"]) == "map" { pkg_meta["metadata"]["name"] } else { () }
}

// Remove the redundant pkg_name segment that appears right after {{instance.appslug}}
// when the Helm chart's fullname pattern is {release}-{pkg} (e.g. cert-manager).
// Must be called after apply_name_to_generated so the placeholder has been resolved.
fn apply_dedup_to_generated(path, pkg_name) {
    if pkg_name == () || pkg_name == "" { return; }
    apply_replace_to_generated(path, `{{instance.appslug}}-${pkg_name}`, "{{instance.appslug}}");
}

// ---------------------------------------------------------------------------
// Image and resource extraction from containers
// ---------------------------------------------------------------------------

// Parse an image string into registry/repository/tag.
// Registry is detected as the first slash-segment that contains a dot or is "localhost".
// If no registry is detected, registry is set to "".
fn parse_image(image_str) {
    let s = `${image_str}`;
    let registry = "";
    let repository = s;
    let tag = ();

    let slash_parts = s.split("/");
    if slash_parts.len() > 1 {
        let first = slash_parts[0];
        if first.contains(".") || first == "localhost" {
            registry = first;
            let rest = [];
            let i = 0;
            for p in slash_parts {
                if i > 0 { rest.push(p); }
                i += 1;
            }
            repository = rest.reduce(|acc, x| if acc == "" { x } else { `${acc}/${x}` }, "");
        }
    }

    // Extract tag as the segment after the last ":"
    let colon_parts = repository.split(":");
    if colon_parts.len() >= 2 {
        tag = colon_parts.pop();
        repository = colon_parts.reduce(|acc, x| if acc == "" { x } else { `${acc}:${x}` }, "");
    }

    #{registry: registry, repository: repository, tag: tag}
}

fn make_img_key(release_name, container_name) {
    let k = `${container_name}`;
    k.replace(" ", "-");
    k.replace("/", "-");
    // Strip the release-name prefix so image keys don't embed the placeholder
    let prefix = `${release_name}-`;
    if k.starts_with(prefix) {
        k.replace(prefix, "");
    } else if k == release_name {
        k = "app";
    }
    if k == "" { k = "app"; }
    k
}

// Strip the release name prefix from a doc name for use in file paths.
// "v3fdf80f5-traefik" + "v3fdf80f5" → "traefik"
fn clean_file_name(doc_name, release_name) {
    clean_file_name(doc_name, release_name, ())
}
fn clean_file_name(doc_name, release_name, ns) {
    let prefix = `${release_name}-`;
    if doc_name.starts_with(prefix) {
        doc_name.replace(prefix, "");
    } else if doc_name == release_name {
        doc_name = "app";
    }
    // Strip the namespace placeholder so file names are stable across regenerations.
    if ns != () && ns != "" {
        doc_name.replace(`-${ns}`, "");
        doc_name.replace(ns, "");
        while doc_name.contains("--") { doc_name.replace("--", "-"); }
    }
    if doc_name == "" { doc_name = "app"; }
    doc_name
}

// Extract images and resources from a container list.
// Returns #{containers, images, resources}.
fn extract_images_resources_from_containers(containers, prefix, release_name) {
    let images = #{};
    let resources = #{};
    let result = [];
    for c in containers {
        let c_name = if "name" in c && c.name != () { make_img_key(release_name, `${c.name}`) } else { "app" };
        let key = if prefix != "" { `${prefix}-${c_name}` } else { c_name };
        if "image" in c && c.image != () && type_of(c.image) == "string" {
            let parsed = parse_image(c.image);
            images[key] = #{registry: parsed.registry, repository: parsed.repository};
            if parsed.tag != () { images[key].tag = parsed.tag; }
            c.image = `IMAGE_CTX_${key}`;
        }
        if "resources" in c && c.resources != () && type_of(c.resources) == "map" {
            resources[key] = c.resources;
            c.resources = `RESOURCES_CTX_${key}`;
        }
        // Extract --*-image=<img> patterns from args
        if "args" in c && c.args != () {
            let new_args = [];
            for arg in c.args {
                let a = `${arg}`;
                if a.contains("-image=") {
                    let parts = a.split("=");
                    if parts.len() >= 2 {
                        let flag = parts[0];
                        let img_str = parts[1];
                        let flag_parts = flag.split("--");
                        // Flag names (--acme-http01-solver-image) don't carry the release prefix
                        let arg_key = make_img_key("", if flag_parts.len() >= 2 { flag_parts[1] } else { flag });
                        let parsed = parse_image(img_str);
                        images[arg_key] = #{registry: parsed.registry, repository: parsed.repository};
                        if parsed.tag != () { images[arg_key].tag = parsed.tag; }
                        a = `${flag}=IMAGE_CTX_${arg_key}`;
                    }
                }
                new_args.push(a);
            }
            c.args = new_args;
        }
        result.push(c);
    }
    #{containers: result, images: images, resources: resources}
}

// Extract images/resources from a workload (Deployment/StatefulSet/Job…).
fn extract_workload_images_resources(doc, release_name) {
    let images = #{};
    let resources = #{};
    if "spec" in doc && doc.spec != () && "template" in doc.spec && doc.spec.template != ()
            && "spec" in doc.spec.template && doc.spec.template.spec != () {
        let spec = doc.spec.template.spec;
        for field in ["containers", "initContainers"] {
            if field in spec && spec[field] != () {
                let prefix = if field == "initContainers" { "init" } else { "" };
                let extr = extract_images_resources_from_containers(spec[field], prefix, release_name);
                spec[field] = extr.containers;
                for k in extr.images.keys() { images[k] = extr.images[k]; }
                for k in extr.resources.keys() { resources[k] = extr.resources[k]; }
            }
        }
        doc.spec.template.spec = spec;
    }
    #{doc: doc, images: images, resources: resources}
}

// Extract images/resources from a CronJob.
fn extract_cronjob_images_resources(doc, release_name) {
    let images = #{};
    let resources = #{};
    if "spec" in doc && doc.spec != () && "jobTemplate" in doc.spec && doc.spec.jobTemplate != ()
            && "spec" in doc.spec.jobTemplate && doc.spec.jobTemplate.spec != ()
            && "template" in doc.spec.jobTemplate.spec && doc.spec.jobTemplate.spec.template != ()
            && "spec" in doc.spec.jobTemplate.spec.template && doc.spec.jobTemplate.spec.template.spec != () {
        let spec = doc.spec.jobTemplate.spec.template.spec;
        for field in ["containers", "initContainers"] {
            if field in spec && spec[field] != () {
                let prefix = if field == "initContainers" { "init" } else { "" };
                let extr = extract_images_resources_from_containers(spec[field], prefix, release_name);
                spec[field] = extr.containers;
                for k in extr.images.keys() { images[k] = extr.images[k]; }
                for k in extr.resources.keys() { resources[k] = extr.resources[k]; }
            }
        }
        doc.spec.jobTemplate.spec.template.spec = spec;
    }
    #{doc: doc, images: images, resources: resources}
}

// Replace IMAGE_CTX_* and RESOURCES_CTX_* markers with HBS expressions in a file.
fn apply_image_resource_expressions(filepath, images_keys, resources_keys) {
    let content = host::file_read(filepath);
    for key in images_keys {
        let marker = `IMAGE_CTX_${key}`;
        let expr = `{{image_from_ctx this "${key}"}}`;
        content.replace(marker, expr);
        content.replace(`"${marker}"`, expr);
        content.replace(`'${marker}'`, expr);
    }
    for key in resources_keys {
        let marker = `RESOURCES_CTX_${key}`;
        let expr = `{{json_to_str (resources_from_ctx this "${key}")}}`;
        content.replace(marker, expr);
        content.replace(`"${marker}"`, expr);
        content.replace(`'${marker}'`, expr);
    }
    host::file_write(filepath, content);
}

// Update package.yaml with extracted images and resources (always overwrites existing entries).
//
// Pure string approach: never re-parse the file through yaml_decode_ordered.
// rust_yaml RoundTrip mode stores per-node span metadata. Block scalars (>, |, >-, |-)
// carry special metadata that makes dump_str produce corrupted output whenever any node
// without that metadata is inserted. String manipulation avoids the issue entirely.
fn update_package_yaml_images_resources(path, images, resources) {
    if images.len() == 0 && resources.len() == 0 { return; }
    let pkg_path = `${path}/package.yaml`;
    if !host::is_file(pkg_path) { return; }

    let nl = "\n";

    // Build new images section text
    let images_text = "";
    if images.len() > 0 {
        images_text = "images:";
        for k in images.keys() {
            let img = images[k];
            images_text = `${images_text}${nl}  ${k}:`;
            images_text = `${images_text}${nl}    registry: ${img["registry"]}`;
            images_text = `${images_text}${nl}    repository: ${img["repository"]}`;
            if type_of(img["tag"]) == "string" && img["tag"] != "" {
                images_text = `${images_text}${nl}    tag: ${img["tag"]}`;
            }
        }
    }

    // Build new resources section text
    let resources_text = "";
    if resources.len() > 0 {
        resources_text = "resources:";
        for k in resources.keys() {
            let res = resources[k];
            resources_text = `${resources_text}${nl}  ${k}:`;
            for phase in ["requests", "limits"] {
                if type_of(res[phase]) == "map" {
                    resources_text = `${resources_text}${nl}    ${phase}:`;
                    for field in res[phase].keys() {
                        resources_text = `${resources_text}${nl}      ${field}: ${res[phase][field]}`;
                    }
                }
            }
        }
    }

    // Read original file and rebuild line by line, replacing existing sections
    let original = host::file_read(pkg_path);
    let lines = original.split(nl);
    let new_lines = [];
    let in_section = false;
    let images_written = false;
    let resources_written = false;

    for line in lines {
        // Top-level key: no leading whitespace, not empty, not comment, not list item
        let is_top = line.len() > 0 && !line.starts_with(" ") && !line.starts_with("\t")
                  && !line.starts_with("#") && !line.starts_with("-");
        if is_top {
            if line.starts_with("images:") {
                if images_text != "" {
                    // Replace section with new content; skip original header and children
                    in_section = true;
                    new_lines.push(images_text);
                    images_written = true;
                    continue;
                }
                // No new images: keep original section as-is
                in_section = false;
            } else if line.starts_with("resources:") {
                if resources_text != "" {
                    in_section = true;
                    new_lines.push(resources_text);
                    resources_written = true;
                    continue;
                }
                in_section = false;
            } else {
                in_section = false;
            }
        }
        if in_section { continue; }
        new_lines.push(line);
    }

    // Append sections not found in original
    if !images_written && images_text != "" { new_lines.push(images_text); }
    if !resources_written && resources_text != "" { new_lines.push(resources_text); }

    // Join lines preserving original line endings
    let content = new_lines.reduce(|acc, x| if acc == "" { x } else { `${acc}${nl}${x}` }, "");

    // Ensure document separator
    if !content.starts_with("---") {
        content = `---${nl}${content}`;
    }

    host::file_write(pkg_path, content);
}

// ---------------------------------------------------------------------------
// Metadata helpers
// ---------------------------------------------------------------------------

fn replace_label_values(labels, name) {
    let result = #{};
    for k in labels.keys() {
        let v = labels[k];
        if type_of(v) == "string" {
            v.replace(name, "{{instance.appslug}}");
            if v == "Helm" { v = "vynil"; }
        }
        result[k] = v;
    }
    result
}

fn clean_metadata(metadata, name) {
    // Clean Helm-specific annotations
    if "annotations" in metadata && metadata.annotations != () {
        let to_remove = metadata.annotations.keys().filter(|k| k.contains("helm.sh") || k.contains("checksum/"));
        for k in to_remove {
            metadata.annotations.remove(k);
        }
        if metadata.annotations.len() == 0 {
            metadata.remove("annotations");
        }
    }
    if "labels" in metadata {
        metadata.remove("labels");
    }
    metadata
}

// Kept for backward compat (called without name in some paths)
fn clean_annotations(metadata) {
    if "annotations" in metadata && metadata.annotations != () {
        let to_remove = metadata.annotations.keys().filter(|k| k.contains("helm.sh") || k.contains("checksum/"));
        for k in to_remove {
            metadata.annotations.remove(k);
        }
        if metadata.annotations.len() == 0 {
            metadata.remove("annotations");
        }
    }
    metadata
}

// ---------------------------------------------------------------------------
// Pod spec - name replacement
// ---------------------------------------------------------------------------

fn replace_image_pull_secrets(ips, name) {
    let result = [];
    for s in ips {
        if "name" in s && s.name != () {
            s.name.replace(name, "{{instance.appslug}}");
        }
        result.push(s);
    }
    result
}

fn replace_volumes(volumes, name) {
    let result = [];
    for v in volumes {
        if "configMap" in v && v.configMap != () && "name" in v.configMap {
            v.configMap.name.replace(name, "{{instance.appslug}}");
        }
        if "secret" in v && v.secret != () && "secretName" in v.secret {
            v.secret.secretName.replace(name, "{{instance.appslug}}");
        }
        if "persistentVolumeClaim" in v && v.persistentVolumeClaim != () && "claimName" in v.persistentVolumeClaim {
            v.persistentVolumeClaim.claimName.replace(name, "{{instance.appslug}}");
        }
        if "projected" in v && v.projected != () && "sources" in v.projected {
            let srcs = [];
            for src in v.projected.sources {
                if "configMap" in src && src.configMap != () && "name" in src.configMap {
                    src.configMap.name.replace(name, "{{instance.appslug}}");
                }
                if "secret" in src && src.secret != () && "name" in src.secret {
                    src.secret.name.replace(name, "{{instance.appslug}}");
                }
                srcs.push(src);
            }
            v.projected.sources = srcs;
        }
        result.push(v);
    }
    result
}

fn replace_containers(containers, name) {
    let result = [];
    for c in containers {
        // Strip release-name prefix from container name (e.g. "v3fdf80f5-traefik" → "traefik")
        if "name" in c && c.name != () {
            c.name = clean_file_name(`${c.name}`, name);
        }
        if "envFrom" in c && c.envFrom != () {
            let ef_list = [];
            for ef in c.envFrom {
                if "configMapRef" in ef && ef.configMapRef != () && "name" in ef.configMapRef {
                    ef.configMapRef.name.replace(name, "{{instance.appslug}}");
                }
                if "secretRef" in ef && ef.secretRef != () && "name" in ef.secretRef {
                    ef.secretRef.name.replace(name, "{{instance.appslug}}");
                }
                ef_list.push(ef);
            }
            c.envFrom = ef_list;
        }
        if "env" in c && c.env != () {
            let env_list = [];
            for e in c.env {
                if "valueFrom" in e && e.valueFrom != () {
                    if "configMapKeyRef" in e.valueFrom && e.valueFrom.configMapKeyRef != () && "name" in e.valueFrom.configMapKeyRef {
                        e.valueFrom.configMapKeyRef.name.replace(name, "{{instance.appslug}}");
                    }
                    if "secretKeyRef" in e.valueFrom && e.valueFrom.secretKeyRef != () && "name" in e.valueFrom.secretKeyRef {
                        e.valueFrom.secretKeyRef.name.replace(name, "{{instance.appslug}}");
                    }
                } else if "value" in e && e.value != () {
                    // Replace name in plain values too (e.g. app URL containing release name)
                    let v = `${e.value}`;
                    v.replace(name, "{{instance.appslug}}");
                    e.value = v;
                }
                env_list.push(e);
            }
            c.env = env_list;
        }
        // Replace release name in all arg values (not just --*-image= ones)
        if "args" in c && c.args != () {
            let new_args = [];
            for arg in c.args {
                let a = `${arg}`;
                a.replace(name, "{{instance.appslug}}");
                new_args.push(a);
            }
            c.args = new_args;
        }
        result.push(c);
    }
    result
}

fn replace_pod_spec(spec, name) {
    if "serviceAccountName" in spec && spec.serviceAccountName != () {
        spec.serviceAccountName.replace(name, "{{instance.appslug}}");
    }
    if "imagePullSecrets" in spec && spec.imagePullSecrets != () {
        spec.imagePullSecrets = replace_image_pull_secrets(spec.imagePullSecrets, name);
    }
    if "volumes" in spec && spec.volumes != () {
        spec.volumes = replace_volumes(spec.volumes, name);
    }
    if "containers" in spec && spec.containers != () {
        spec.containers = replace_containers(spec.containers, name);
    }
    if "initContainers" in spec && spec.initContainers != () {
        spec.initContainers = replace_containers(spec.initContainers, name);
    }
    spec
}

fn replace_pod_template(tmpl, name) {
    if "metadata" in tmpl && tmpl.metadata != () {
        tmpl.metadata = clean_metadata(tmpl.metadata, name);
    }
    if "spec" in tmpl && tmpl.spec != () {
        tmpl.spec = replace_pod_spec(tmpl.spec, name);
    }
    tmpl
}

// ---------------------------------------------------------------------------
// Security contexts (added only when absent)
// ---------------------------------------------------------------------------

fn add_security_contexts(spec) {
    if !("securityContext" in spec) || spec.securityContext == () {
        spec.securityContext = #{
            runAsNonRoot: true,
            seccompProfile: #{"type": "RuntimeDefault"},
        };
    }
    for field in ["containers", "initContainers"] {
        if field in spec && spec[field] != () {
            let ctrs = [];
            for c in spec[field] {
                if !("securityContext" in c) || c.securityContext == () {
                    c.securityContext = #{
                        allowPrivilegeEscalation: false,
                        readOnlyRootFilesystem: true,
                        runAsNonRoot: true,
                        capabilities: #{drop: ["ALL"]},
                    };
                }
                ctrs.push(c);
            }
            spec[field] = ctrs;
        }
    }
    spec
}

// ---------------------------------------------------------------------------
// Topology spread constraints (replaces / supplements podAntiAffinity)
// ---------------------------------------------------------------------------

fn convert_affinity_to_tsc(spec, pod_labels) {
    // If TSC already present, just clean up redundant podAntiAffinity
    if "topologySpreadConstraints" in spec && spec.topologySpreadConstraints != () {
        if "affinity" in spec && spec.affinity != () && "podAntiAffinity" in spec.affinity {
            spec.affinity.remove("podAntiAffinity");
            if spec.affinity.len() == 0 {
                spec.remove("affinity");
            }
        }
        return spec;
    }

    let topology_key = "kubernetes.io/hostname";
    let match_labels = #{};

    // Try to extract topology key and labelSelector from existing podAntiAffinity
    if "affinity" in spec && spec.affinity != () && "podAntiAffinity" in spec.affinity {
        let paa = spec.affinity.podAntiAffinity;
        let terms = [];
        if "preferredDuringSchedulingIgnoredDuringExecution" in paa && paa.preferredDuringSchedulingIgnoredDuringExecution != () {
            for item in paa.preferredDuringSchedulingIgnoredDuringExecution {
                if "podAffinityTerm" in item { terms.push(item.podAffinityTerm); }
            }
        }
        if "requiredDuringSchedulingIgnoredDuringExecution" in paa && paa.requiredDuringSchedulingIgnoredDuringExecution != () {
            for item in paa.requiredDuringSchedulingIgnoredDuringExecution {
                terms.push(item);
            }
        }
        if terms.len() > 0 {
            if "topologyKey" in terms[0] && terms[0].topologyKey != () {
                topology_key = terms[0].topologyKey;
            }
            if "labelSelector" in terms[0] && terms[0].labelSelector != () && "matchLabels" in terms[0].labelSelector && terms[0].labelSelector.matchLabels != () {
                match_labels = terms[0].labelSelector.matchLabels;
            }
        }
        spec.affinity.remove("podAntiAffinity");
        if spec.affinity.len() == 0 {
            spec.remove("affinity");
        }
    }

    // Fallback: use identifying labels from pod template
    if match_labels.len() == 0 {
        for key in ["app.kubernetes.io/instance", "app.kubernetes.io/name", "app"] {
            if key in pod_labels && pod_labels[key] != () {
                match_labels[key] = pod_labels[key];
            }
        }
    }
    // Last resort
    if match_labels.len() == 0 {
        match_labels["app.kubernetes.io/instance"] = "{{instance.appslug}}";
    }

    spec.topologySpreadConstraints = [#{
        maxSkew: 1,
        topologyKey: topology_key,
        whenUnsatisfiable: "ScheduleAnyway",
        labelSelector: #{matchLabels: match_labels},
    }];
    spec
}

// ---------------------------------------------------------------------------
// Env var extraction → dedicated ConfigMap
// ---------------------------------------------------------------------------

fn extract_env_configmap(path, doc_name, doc, name) {
    if !("template" in doc.spec) || doc.spec.template == () { return doc; }
    if !("spec" in doc.spec.template) || doc.spec.template.spec == () { return doc; }

    // Collect all plain-value env vars from all containers
    let env_data = #{};
    for field in ["containers", "initContainers"] {
        if field in doc.spec.template.spec && doc.spec.template.spec[field] != () {
            for c in doc.spec.template.spec[field] {
                if "env" in c && c.env != () {
                    for e in c.env {
                        if !("valueFrom" in e) && "value" in e && e.value != () {
                            env_data[e.name] = `${e.value}`;
                        }
                    }
                }
            }
        }
    }
    if env_data.len() == 0 { return doc; }

    // Write ConfigMap file
    let cm_name = `${doc.metadata.name}-envs`;
    let cm = #{
        apiVersion: "v1",
        kind: "ConfigMap",
        metadata: #{name: cm_name},
        data: env_data,
    };
    let cm_file_name = clean_file_name(doc_name, name);
    host::create_dir(`${path}/get_others`);
    gen_yaml(`${path}/get_others/ConfigMap_${cm_file_name}-envs.yaml.hbs`, cm);

    // Update containers: remove plain envs, prepend envFrom reference
    let spec = doc.spec.template.spec;
    for field in ["containers", "initContainers"] {
        if field in spec && spec[field] != () {
            let ctrs = [];
            for c in spec[field] {
                let had_plain = false;
                if "env" in c && c.env != () {
                    had_plain = c.env.filter(|e| !("valueFrom" in e) && "value" in e && e.value != ()).len() > 0;
                    let new_env = c.env.filter(|e| "valueFrom" in e);
                    if new_env.len() > 0 {
                        c.env = new_env;
                    } else {
                        c.remove("env");
                    }
                }
                if had_plain {
                    let cm_ref = #{configMapRef: #{name: cm_name}};
                    if "envFrom" in c && c.envFrom != () {
                        let new_ef = [cm_ref];
                        for ef in c.envFrom { new_ef.push(ef); }
                        c.envFrom = new_ef;
                    } else {
                        c.envFrom = [cm_ref];
                    }
                }
                ctrs.push(c);
            }
            spec[field] = ctrs;
        }
    }
    doc.spec.template.spec = spec;
    doc
}

// ---------------------------------------------------------------------------
// Reloader annotations
// ---------------------------------------------------------------------------

fn add_reloader_annotations(metadata, spec) {
    let cms = [];
    let secrets = [];

    // Volumes
    if "volumes" in spec && spec.volumes != () {
        for v in spec.volumes {
            if "configMap" in v && v.configMap != () && "name" in v.configMap && v.configMap.name != () {
                if !cms.contains(v.configMap.name) { cms.push(v.configMap.name); }
            }
            if "secret" in v && v.secret != () && "secretName" in v.secret && v.secret.secretName != () {
                if !secrets.contains(v.secret.secretName) { secrets.push(v.secret.secretName); }
            }
            if "projected" in v && v.projected != () && "sources" in v.projected && v.projected.sources != () {
                for src in v.projected.sources {
                    if "configMap" in src && src.configMap != () && "name" in src.configMap {
                        if !cms.contains(src.configMap.name) { cms.push(src.configMap.name); }
                    }
                    if "secret" in src && src.secret != () && "name" in src.secret {
                        if !secrets.contains(src.secret.name) { secrets.push(src.secret.name); }
                    }
                }
            }
        }
    }
    // Containers + initContainers
    for field in ["containers", "initContainers"] {
        if field in spec && spec[field] != () {
            for c in spec[field] {
                if "envFrom" in c && c.envFrom != () {
                    for ef in c.envFrom {
                        if "configMapRef" in ef && ef.configMapRef != () && "name" in ef.configMapRef {
                            if !cms.contains(ef.configMapRef.name) { cms.push(ef.configMapRef.name); }
                        }
                        if "secretRef" in ef && ef.secretRef != () && "name" in ef.secretRef {
                            if !secrets.contains(ef.secretRef.name) { secrets.push(ef.secretRef.name); }
                        }
                    }
                }
                if "env" in c && c.env != () {
                    for e in c.env {
                        if "valueFrom" in e && e.valueFrom != () {
                            if "configMapKeyRef" in e.valueFrom && e.valueFrom.configMapKeyRef != () && "name" in e.valueFrom.configMapKeyRef {
                                if !cms.contains(e.valueFrom.configMapKeyRef.name) { cms.push(e.valueFrom.configMapKeyRef.name); }
                            }
                            if "secretKeyRef" in e.valueFrom && e.valueFrom.secretKeyRef != () && "name" in e.valueFrom.secretKeyRef {
                                if !secrets.contains(e.valueFrom.secretKeyRef.name) { secrets.push(e.valueFrom.secretKeyRef.name); }
                            }
                        }
                    }
                }
            }
        }
    }

    if cms.len() > 0 || secrets.len() > 0 {
        if !("annotations" in metadata) || metadata.annotations == () {
            metadata.annotations = #{};
        }
        if cms.len() > 0 {
            metadata.annotations["configmap.reloader.stakater.com/reload"] = cms.reduce(|acc, x| if acc != "" { `${acc},${x}` } else { x }, "");
        }
        if secrets.len() > 0 {
            metadata.annotations["secret.reloader.stakater.com/reload"] = secrets.reduce(|acc, x| if acc != "" { `${acc},${x}` } else { x }, "");
        }
    }
    metadata
}

// ---------------------------------------------------------------------------
// Binding / webhook helpers
// ---------------------------------------------------------------------------

fn replace_binding_impl(doc, name, roleref_prefix) {
    if "subjects" in doc && doc.subjects != () {
        let subj = [];
        for s in doc.subjects {
            s.name.replace(name, "{{instance.appslug}}");
            s.namespace = "{{instance.namespace}}";
            subj.push(s);
        }
        doc.subjects = subj;
    }
    if "roleRef" in doc && doc.roleRef != () {
        doc.roleRef.name.replace(name, roleref_prefix);
        if "namespace" in doc.roleRef && doc.roleRef.namespace != () {
            doc.roleRef.namespace = "{{instance.namespace}}";
        }
    }
    doc
}

// RoleBinding : roleRef pointe un Role namespace-scoped → pas de préfixe namespace.
fn replace_binding(doc, name) {
    replace_binding_impl(doc, name, "{{instance.appslug}}")
}
// ClusterRoleBinding : roleRef doit matcher le metadata.name du ClusterRole,
// qui est préfixé par {{instance.namespace}}-{{instance.appslug}} dans gen_system.
fn replace_cluster_binding(doc, name) {
    replace_binding_impl(doc, name, "{{instance.namespace}}-{{instance.appslug}}")
}

fn replace_webhooks(doc, name) {
    if "webhooks" in doc && doc.webhooks != () {
        let webhooks = [];
        for w in doc.webhooks {
            if "clientConfig" in w && w.clientConfig != () && "service" in w.clientConfig && w.clientConfig.service != () {
                w.clientConfig.service.name.replace(name, "{{instance.appslug}}");
                w.clientConfig.service.namespace = "{{instance.namespace}}";
            }
            webhooks.push(w);
        }
        doc.webhooks = webhooks;
    }
    doc
}

// ---------------------------------------------------------------------------
// Ingress
// ---------------------------------------------------------------------------

fn replace_ingress(doc, name) {
    doc.metadata = clean_metadata(doc.metadata, name);
    if "rules" in doc.spec && doc.spec.rules != () {
        let rules = [];
        for rule in doc.spec.rules {
            if "http" in rule && rule.http != () && "paths" in rule.http {
                let paths = [];
                for p in rule.http.paths {
                    if "backend" in p && p.backend != () && "service" in p.backend && p.backend.service != () && "name" in p.backend.service {
                        p.backend.service.name.replace(name, "{{instance.appslug}}");
                    }
                    paths.push(p);
                }
                rule.http.paths = paths;
            }
            rules.push(rule);
        }
        doc.spec.rules = rules;
    }
    if "tls" in doc.spec && doc.spec.tls != () {
        let tls_list = [];
        for tls in doc.spec.tls {
            if "secretName" in tls && tls.secretName != () {
                tls.secretName.replace(name, "{{instance.appslug}}");
            }
            tls_list.push(tls);
        }
        doc.spec.tls = tls_list;
    }
    doc
}

// ---------------------------------------------------------------------------
// Full workload processing (Deployment / StatefulSet / DaemonSet / ReplicaSet)
// ---------------------------------------------------------------------------

fn replace_workload(path, doc_name, doc, name) {
    replace_workload(path, doc_name, doc, name, true)
}
fn replace_workload(path, doc_name, doc, name, extract_envs) {
    doc.metadata = clean_metadata(doc.metadata, name);

    if "template" in doc.spec && doc.spec.template != () {
        doc.spec.template = replace_pod_template(doc.spec.template, name);

        let pod_labels = #{};
        if "metadata" in doc.spec.template && doc.spec.template.metadata != () && "labels" in doc.spec.template.metadata {
            pod_labels = doc.spec.template.metadata.labels;
        }

        let spec = doc.spec.template.spec;
        spec = add_security_contexts(spec);
        spec = convert_affinity_to_tsc(spec, pod_labels);
        doc.spec.template.spec = spec;
    }

    // StatefulSet: serviceName and volumeClaimTemplates
    if "kind" in doc && doc.kind == "StatefulSet" {
        if "serviceName" in doc.spec && doc.spec.serviceName != () {
            doc.spec.serviceName.replace(name, "{{instance.appslug}}");
        }
        if "volumeClaimTemplates" in doc.spec && doc.spec.volumeClaimTemplates != () {
            let vcl = [];
            for vct in doc.spec.volumeClaimTemplates {
                if "metadata" in vct && vct.metadata != () && "name" in vct.metadata {
                    vct.metadata.name.replace(name, "{{instance.appslug}}");
                }
                vcl.push(vct);
            }
            doc.spec.volumeClaimTemplates = vcl;
        }
    }

    if extract_envs {
        doc = extract_env_configmap(path, doc_name, doc, name);
    }

    if "template" in doc.spec && doc.spec.template != () && "spec" in doc.spec.template {
        doc.metadata = add_reloader_annotations(doc.metadata, doc.spec.template.spec);
    }
    doc
}

// ---------------------------------------------------------------------------
// Job / CronJob (security + env extraction + reloader, no TSC)
// ---------------------------------------------------------------------------

fn replace_job(path, doc_name, doc, name) {
    replace_job(path, doc_name, doc, name, true)
}
fn replace_job(path, doc_name, doc, name, extract_envs) {
    doc.metadata = clean_metadata(doc.metadata, name);
    if "template" in doc.spec && doc.spec.template != () {
        doc.spec.template = replace_pod_template(doc.spec.template, name);
        doc.spec.template.spec = add_security_contexts(doc.spec.template.spec);
    }
    if extract_envs {
        doc = extract_env_configmap(path, doc_name, doc, name);
    }
    if "template" in doc.spec && doc.spec.template != () && "spec" in doc.spec.template {
        doc.metadata = add_reloader_annotations(doc.metadata, doc.spec.template.spec);
    }
    doc
}

fn replace_cronjob(path, doc_name, doc, name) {
    doc.metadata = clean_metadata(doc.metadata, name);
    if "jobTemplate" in doc.spec && doc.spec.jobTemplate != () && "spec" in doc.spec.jobTemplate && doc.spec.jobTemplate.spec != () {
        let jt = doc.spec.jobTemplate;
        if "template" in jt.spec && jt.spec.template != () {
            jt.spec.template = replace_pod_template(jt.spec.template, name);
            jt.spec.template.spec = add_security_contexts(jt.spec.template.spec);
        }
        doc.spec.jobTemplate = jt;
    }
    if "jobTemplate" in doc.spec && doc.spec.jobTemplate != () && "spec" in doc.spec.jobTemplate && "template" in doc.spec.jobTemplate.spec {
        let pod_spec = doc.spec.jobTemplate.spec.template.spec;
        if pod_spec != () {
            doc.metadata = add_reloader_annotations(doc.metadata, pod_spec);
        }
    }
    doc
}

// ---------------------------------------------------------------------------
// Selector / label analysis and replacement
// ---------------------------------------------------------------------------

// Returns true if all keys of selector are present with equal values in labels
fn selector_matches(selector, labels) {
    if selector.len() == 0 { return false; }
    for k in selector.keys() {
        if !(k in labels) || `${labels[k]}` != `${selector[k]}` {
            return false;
        }
    }
    true
}

// Strip the release name prefix from a label value to get the component name
// e.g. "myapp-server" with name "myapp" → "server"
fn extract_comp(val, name) {
    let s = `${val}`;
    let prefix = `${name}-`;
    if s.starts_with(prefix) && s.len() > prefix.len() {
        let parts = s.split(prefix);
        if parts.len() >= 2 && parts[0] == "" {
            s = parts[1];
        }
    }
    s
}

// Find which label key differentiates multiple workloads (all have it, all values unique)
fn find_diff_key(workloads) {
    for key in ["app.kubernetes.io/component", "component", "app"] {
        let vals = [];
        let all_have = true;
        for wl in workloads {
            if key in wl.labels && wl.labels[key] != () {
                vals.push(`${wl.labels[key]}`);
            } else {
                all_have = false;
            }
        }
        if !all_have { continue; }
        let seen = [];
        let unique = true;
        for v in vals {
            if seen.contains(v) { unique = false; break; }
            seen.push(v);
        }
        if unique { return key; }
    }
    ""
}

// Pre-analyse docs to build comp_map (workload orig_name → comp) and svc_comp_map
fn build_analysis(docs, name) {
    let workloads = [];
    let services = [];

    for doc in docs.filter(|obj| type_of(obj) == "map" && "kind" in obj && "metadata" in obj && type_of(obj.metadata) == "map" && obj.kind != ()) {
        let kind = doc.kind;
        let orig_name = doc.metadata.name;

        if ["Deployment", "StatefulSet", "DaemonSet", "ReplicaSet"].contains(kind) {
            let tmpl_labels = #{};
            if "spec" in doc && doc.spec != () && "template" in doc.spec && doc.spec.template != () && "metadata" in doc.spec.template && doc.spec.template.metadata != () && "labels" in doc.spec.template.metadata && doc.spec.template.metadata.labels != () {
                tmpl_labels = doc.spec.template.metadata.labels;
            }
            workloads.push(#{orig_name: orig_name, kind: kind, labels: tmpl_labels});
        }
        if kind == "Service" {
            let selector = #{};
            if "spec" in doc && doc.spec != () && "selector" in doc.spec && doc.spec.selector != () {
                selector = doc.spec.selector;
            }
            let svc_labels = #{};
            if "metadata" in doc && doc.metadata != () && "labels" in doc.metadata && doc.metadata.labels != () {
                svc_labels = doc.metadata.labels;
            }
            services.push(#{orig_name: orig_name, selector: selector, svc_labels: svc_labels});
        }
    }

    // Build service → workload pairs via selector matching
    let pairs = [];
    for svc in services {
        for wl in workloads {
            if selector_matches(svc.selector, wl.labels) {
                pairs.push(#{svc: svc.orig_name, workload: wl.orig_name});
            }
        }
    }

    // Build comp_map: workload orig_name → comp string
    let comp_map = #{};
    if workloads.len() == 1 {
        comp_map[workloads[0].orig_name] = "app";
    } else if workloads.len() > 1 {
        let diff_key = find_diff_key(workloads);
        for wl in workloads {
            let comp = "app";
            if diff_key != "" && diff_key in wl.labels && wl.labels[diff_key] != () {
                comp = extract_comp(`${wl.labels[diff_key]}`, name);
            } else {
                let suffix = extract_comp(wl.orig_name, name);
                if suffix != wl.orig_name { comp = suffix; }
            }
            comp_map[wl.orig_name] = comp;
        }
    }

    // Build svc_comp_map: service orig_name → comp string (from paired workload)
    let svc_comp_map = #{};
    for svc in services {
        let comp = "app";
        for pair in pairs.filter(|p| p.svc == svc.orig_name) {
            if pair.workload in comp_map {
                comp = comp_map[pair.workload];
            }
            break;
        }
        svc_comp_map[svc.orig_name] = comp;
    }

    #{workloads: workloads, services: services, pairs: pairs, comp_map: comp_map, svc_comp_map: svc_comp_map}
}

// Find comp for a resource whose selector targets workload pods directly (PDB, NetworkPolicy)
fn find_comp_for_selector(analysis, selector) {
    for wl in analysis.workloads {
        if selector_matches(selector, wl.labels) && wl.orig_name in analysis.comp_map {
            return analysis.comp_map[wl.orig_name];
        }
    }
    ""
}

// Find comp for a ServiceMonitor whose spec.selector targets Service labels
fn find_comp_for_svc_labels(analysis, sm_selector) {
    for svc in analysis.services {
        if selector_matches(sm_selector, svc.svc_labels) && svc.orig_name in analysis.svc_comp_map {
            return analysis.svc_comp_map[svc.orig_name];
        }
    }
    // Fallback: try matching against workload labels directly
    for wl in analysis.workloads {
        if selector_matches(sm_selector, wl.labels) && wl.orig_name in analysis.comp_map {
            return analysis.comp_map[wl.orig_name];
        }
    }
    ""
}

// Set marker strings in workload doc for selector/label fields
fn apply_workload_selectors(doc, comp) {
    let marker = `SELECTOR_COMP_${comp}`;
    if "spec" in doc && doc.spec != () {
        if "selector" in doc.spec && doc.spec.selector != () && "matchLabels" in doc.spec.selector {
            doc.spec.selector.matchLabels = marker;
        }
        if "template" in doc.spec && doc.spec.template != () {
            if "metadata" in doc.spec.template && doc.spec.template.metadata != () {
                doc.spec.template.metadata.labels = "LABELS_FROM_CTX";
            }
            // TSC vit dans le pod spec (spec.template.spec), pas dans le workload spec (spec).
            // Extraire le pod_spec pour éviter les problèmes d'assignation profonde.
            if "spec" in doc.spec.template && doc.spec.template.spec != () {
                let pod_spec = doc.spec.template.spec;
                if "topologySpreadConstraints" in pod_spec && pod_spec.topologySpreadConstraints != () {
                    let tscs = [];
                    for tsc in pod_spec.topologySpreadConstraints {
                        if "labelSelector" in tsc && tsc.labelSelector != () && "matchLabels" in tsc.labelSelector {
                            tsc.labelSelector = #{matchLabels: marker};
                        }
                        tscs.push(tsc);
                    }
                    pod_spec.topologySpreadConstraints = tscs;
                }
                doc.spec.template.spec = pod_spec;
            }
        }
    }
    doc
}

// Set marker strings in a Service doc
fn apply_svc_selectors(doc, comp) {
    if "spec" in doc && doc.spec != () && "selector" in doc.spec {
        doc.spec.selector = `SELECTOR_COMP_${comp}`;
    }
    doc
}

// Post-process a generated file: replace marker strings with HBS expressions
fn apply_selector_expressions(filepath, comp) {
    let content = host::file_read(filepath);
    let sel_marker = `SELECTOR_COMP_${comp}`;
    let sel_expr = `{{json_to_str (selector_from_ctx this comp="${comp}")}}`;
    let lbl_expr = `{{json_to_str (labels_from_ctx this comp="${comp}")}}`;
    // Unquoted markers (yaml_encode normally won't quote these safe strings)
    content.replace(sel_marker, sel_expr);
    content.replace("LABELS_FROM_CTX", lbl_expr);
    // Quoted variants (in case yaml_encode wrapped them)
    content.replace(`"${sel_marker}"`, sel_expr);
    content.replace(`'${sel_marker}'`, sel_expr);
    content.replace(`"LABELS_FROM_CTX"`, lbl_expr);
    content.replace(`'LABELS_FROM_CTX'`, lbl_expr);
    host::file_write(filepath, content);
}

// ---------------------------------------------------------------------------
// gen_system
// ---------------------------------------------------------------------------

fn gen_system_core(path, docs, name, ns) {
    if name != () {
        let analysis = build_analysis(docs, name);
        let all_images = #{};
        let all_resources = #{};
        for doc in docs.filter(|obj| type_of(obj) == "map" && obj.keys().contains("kind") && obj.keys().contains("metadata") && type_of(obj.metadata) == "map") {
            if doc.kind == "CustomResourceDefinition" {
                host::create_dir(`${path}/get_crds`);
                doc.metadata = clean_metadata(doc.metadata, name);
                if "conversion" in doc.spec && doc.spec.conversion != () {
                    if "webhook" in doc.spec.conversion && doc.spec.conversion.webhook != () {
                        doc.spec.conversion.webhook.clientConfig.service.name.replace(name, "{{instance.appslug}}");
                        doc.spec.conversion.webhook.clientConfig.service.namespace = "{{instance.namespace}}";
                    }
                    gen_crd_yaml(`${path}/get_crds/${doc.metadata.name}.yaml.hbs`, doc);
                } else {
                    gen_crd_yaml(`${path}/get_crds/${doc.metadata.name}.yaml`, doc);
                }
            } else if doc.kind != () && doc.kind != "Namespace" {
                host::create_dir(`${path}/get_systems`);
                let doc_name = doc.metadata.name;   // original — used for analysis lookups
                let file_name = clean_file_name(`${doc_name}`, name, ns);  // sanitized for file path
                if doc.kind == "ClusterRoleBinding" || doc.kind == "ClusterRole" || doc.kind == "ValidatingWebhookConfiguration" || doc.kind == "MutatingWebhookConfiguration" || doc.kind == "APIService" {
                    doc.metadata.name = replace_first(`${doc.metadata.name}`, name, "{{instance.namespace}}-{{instance.appslug}}");
                    // Strip ns placeholder: il est déjà dans le préfixe {{instance.namespace}}-
                    if ns != () && ns != "" {
                        doc.metadata.name.replace(`-${ns}`, "");
                        doc.metadata.name.replace(ns, "");
                    }
                } else {
                    doc.metadata.name = replace_first(`${doc.metadata.name}`, name, "{{instance.appslug}}");
                }
                if "namespace" in doc.metadata {
                    doc.metadata.remove("namespace");
                }
                doc.metadata = clean_metadata(doc.metadata, name);
                if doc.kind == "ClusterRoleBinding" {
                    doc = replace_cluster_binding(doc, name);
                    // Aligner le roleRef sur le même nettoyage ns
                    if ns != () && ns != "" && "roleRef" in doc && doc.roleRef != () {
                        doc.roleRef.name.replace(`-${ns}`, "");
                        doc.roleRef.name.replace(ns, "");
                    }
                } else if doc.kind == "RoleBinding" {
                    doc = replace_binding(doc, name);
                }
                if doc.kind == "ValidatingWebhookConfiguration" || doc.kind == "MutatingWebhookConfiguration" {
                    doc = replace_webhooks(doc, name);
                }
                if doc.kind == "APIService" {
                    doc.spec.service.name.replace(name, "{{instance.appslug}}");
                    doc.spec.service.namespace = "{{instance.namespace}}";
                }
                if ["Deployment", "ReplicaSet", "StatefulSet", "DaemonSet"].contains(doc.kind) {
                    let extr = extract_workload_images_resources(doc, name);
                    doc = extr.doc;
                    for k in extr.images.keys() { all_images[k] = extr.images[k]; }
                    for k in extr.resources.keys() { all_resources[k] = extr.resources[k]; }
                    doc = replace_workload(path, doc_name, doc, name, false);
                    let comp = if doc_name in analysis.comp_map { analysis.comp_map[doc_name] } else { "" };
                    if comp != "" { doc = apply_workload_selectors(doc, comp); }
                    let fp = `${path}/get_systems/${doc.kind}_${file_name}.yaml.hbs`;
                    gen_yaml(fp, doc);
                    if extr.images.len() > 0 || extr.resources.len() > 0 {
                        apply_image_resource_expressions(fp, extr.images.keys(), extr.resources.keys());
                    }
                    if comp != "" { apply_selector_expressions(fp, comp); }
                } else if doc.kind == "Job" {
                    let extr = extract_workload_images_resources(doc, name);
                    doc = extr.doc;
                    for k in extr.images.keys() { all_images[k] = extr.images[k]; }
                    for k in extr.resources.keys() { all_resources[k] = extr.resources[k]; }
                    doc = replace_job(path, doc_name, doc, name, false);
                    let fp = `${path}/get_systems/${doc.kind}_${file_name}.yaml.hbs`;
                    gen_yaml(fp, doc);
                    if extr.images.len() > 0 || extr.resources.len() > 0 {
                        apply_image_resource_expressions(fp, extr.images.keys(), extr.resources.keys());
                    }
                } else if doc.kind == "CronJob" {
                    let extr = extract_cronjob_images_resources(doc, name);
                    doc = extr.doc;
                    for k in extr.images.keys() { all_images[k] = extr.images[k]; }
                    for k in extr.resources.keys() { all_resources[k] = extr.resources[k]; }
                    doc = replace_cronjob(path, doc_name, doc, name);
                    let fp = `${path}/get_systems/${doc.kind}_${file_name}.yaml.hbs`;
                    gen_yaml(fp, doc);
                    if extr.images.len() > 0 || extr.resources.len() > 0 {
                        apply_image_resource_expressions(fp, extr.images.keys(), extr.resources.keys());
                    }
                } else if doc.kind == "HorizontalPodAutoscaler" && "scaleTargetRef" in doc.spec && doc.spec.scaleTargetRef != () {
                    doc.spec.scaleTargetRef.name.replace(name, "{{instance.appslug}}");
                    gen_yaml(`${path}/get_systems/${doc.kind}_${file_name}.yaml.hbs`, doc);
                } else if doc.kind == "Service" && doc_name in analysis.svc_comp_map {
                    let comp = analysis.svc_comp_map[doc_name];
                    doc = apply_svc_selectors(doc, comp);
                    let fp = `${path}/get_systems/${doc.kind}_${file_name}.yaml.hbs`;
                    gen_yaml(fp, doc);
                    apply_selector_expressions(fp, comp);
                } else if doc.kind == "ServiceMonitor" {
                    let fp = `${path}/get_systems/${doc.kind}_${file_name}.yaml.hbs`;
                    let res_comp = "";
                    if "spec" in doc && doc.spec != () && "selector" in doc.spec && doc.spec.selector != () && "matchLabels" in doc.spec.selector && doc.spec.selector.matchLabels != () {
                        res_comp = find_comp_for_svc_labels(analysis, doc.spec.selector.matchLabels);
                        if res_comp != "" {
                            doc.spec.selector.matchLabels = `SELECTOR_COMP_${res_comp}`;
                        }
                    }
                    gen_yaml_if(fp, "servicemonitors.monitoring.coreos.com", doc);
                    if res_comp != "" { apply_selector_expressions(fp, res_comp); }
                } else if doc.kind == "PrometheusRule" {
                    gen_yaml_if(`${path}/get_systems/${doc.kind}_${file_name}.yaml.hbs`, "prometheusrules.monitoring.coreos.com", doc);
                } else {
                    // NetworkPolicy and PodDisruptionBudget: replace podSelector/spec.selector matchLabels
                    let res_comp = "";
                    if doc.kind == "NetworkPolicy" && "spec" in doc && doc.spec != () && "podSelector" in doc.spec && doc.spec.podSelector != () && "matchLabels" in doc.spec.podSelector && doc.spec.podSelector.matchLabels != () {
                        res_comp = find_comp_for_selector(analysis, doc.spec.podSelector.matchLabels);
                        if res_comp != "" {
                            doc.spec.podSelector.matchLabels = `SELECTOR_COMP_${res_comp}`;
                        }
                    } else if doc.kind == "PodDisruptionBudget" && "spec" in doc && doc.spec != () && "selector" in doc.spec && doc.spec.selector != () && "matchLabels" in doc.spec.selector && doc.spec.selector.matchLabels != () {
                        res_comp = find_comp_for_selector(analysis, doc.spec.selector.matchLabels);
                        if res_comp != "" {
                            doc.spec.selector.matchLabels = `SELECTOR_COMP_${res_comp}`;
                        }
                    }
                    let fp = `${path}/get_systems/${doc.kind}_${file_name}.yaml.hbs`;
                    gen_yaml(fp, doc);
                    if res_comp != "" { apply_selector_expressions(fp, res_comp); }
                }
            }
        }
        update_package_yaml_images_resources(path, all_images, all_resources);
    }
}
fn gen_system(path, docs, name) {
    gen_system_core(path, docs, name, ());
}
fn gen_system(path, docs) {
    let yaml = core::yaml_decode(host::file_read(path+"/package.yaml"));
    let name = if type_of(yaml["metadata"]) == "map" {yaml["metadata"]["name"]} else {()};
    gen_system_core(path, docs, name, ());
    apply_dedup_to_generated(path, name);
}
fn gen_system(path, docs, name, ns) {
    gen_system_core(path, docs, name, ns);
    apply_name_to_generated(path, name);
    apply_namespace_to_generated(path, ns);
    apply_dedup_to_generated(path, read_pkg_name(path));
}

// ---------------------------------------------------------------------------
// gen_tenant
// ---------------------------------------------------------------------------

fn gen_tenant_core(path, docs, name, ns) {
    if name != () {
        let analysis = build_analysis(docs, name);
        let all_images = #{};
        let all_resources = #{};
        for doc in docs.filter(|obj| type_of(obj) == "map" && obj.keys().contains("kind") && obj.keys().contains("metadata") && type_of(obj.metadata) == "map" && obj.kind != ()) {
            let doc_name = doc.metadata.name;
            let file_name = clean_file_name(`${doc_name}`, name, ns);
            doc.metadata.name = replace_first(`${doc.metadata.name}`, name, "{{instance.appslug}}");
            if "namespace" in doc.metadata {
                doc.metadata.remove("namespace");
            }
            doc.metadata = clean_metadata(doc.metadata, name);
            if ["PersistentVolumeClaim"].contains(doc.kind) {
                host::create_dir(`${path}/get_vitals`);
                gen_yaml(`${path}/get_vitals/${doc.kind}_${file_name}.yaml.hbs`, doc);
            } else if ["Deployment", "ReplicaSet", "StatefulSet", "DaemonSet"].contains(doc.kind) {
                host::create_dir(`${path}/get_scalables`);
                let extr = extract_workload_images_resources(doc, name);
                doc = extr.doc;
                for k in extr.images.keys() { all_images[k] = extr.images[k]; }
                for k in extr.resources.keys() { all_resources[k] = extr.resources[k]; }
                doc = replace_workload(path, doc_name, doc, name);
                let comp = if doc_name in analysis.comp_map { analysis.comp_map[doc_name] } else { "" };
                if comp != "" { doc = apply_workload_selectors(doc, comp); }
                let fp = `${path}/get_scalables/${doc.kind}_${file_name}.yaml.hbs`;
                gen_yaml(fp, doc);
                if extr.images.len() > 0 || extr.resources.len() > 0 {
                    apply_image_resource_expressions(fp, extr.images.keys(), extr.resources.keys());
                }
                if comp != "" { apply_selector_expressions(fp, comp); }
            } else if ["Namespace", "CustomResourceDefinition", "ClusterRole", "ClusterRoleBinding", "ValidatingWebhookConfiguration", "MutatingWebhookConfiguration", "APIService"].contains(doc.kind) {
                host::create_dir(`${path}/get_systems`);
                if doc.kind == "ClusterRoleBinding" {
                    doc = replace_binding(doc, name);
                }
                if doc.kind == "ValidatingWebhookConfiguration" || doc.kind == "MutatingWebhookConfiguration" {
                    doc = replace_webhooks(doc, name);
                }
                if doc.kind == "APIService" {
                    doc.spec.service.name.replace(name, "{{instance.appslug}}");
                    doc.spec.service.namespace = "{{instance.namespace}}";
                }
                gen_yaml(`${path}/get_systems/${doc.kind}_${file_name}.yaml.hbs`, doc);
            } else {
                host::create_dir(`${path}/get_others`);
                if doc.kind == "RoleBinding" {
                    doc = replace_binding(doc, name);
                }
                let img_extr = #{images: #{}, resources: #{}};
                if doc.kind == "Job" {
                    img_extr = extract_workload_images_resources(doc, name);
                    doc = img_extr.doc;
                    for k in img_extr.images.keys() { all_images[k] = img_extr.images[k]; }
                    for k in img_extr.resources.keys() { all_resources[k] = img_extr.resources[k]; }
                    doc = replace_job(path, doc_name, doc, name);
                }
                if doc.kind == "CronJob" {
                    img_extr = extract_cronjob_images_resources(doc, name);
                    doc = img_extr.doc;
                    for k in img_extr.images.keys() { all_images[k] = img_extr.images[k]; }
                    for k in img_extr.resources.keys() { all_resources[k] = img_extr.resources[k]; }
                    doc = replace_cronjob(path, doc_name, doc, name);
                }
                if doc.kind == "HorizontalPodAutoscaler" && "scaleTargetRef" in doc.spec && doc.spec.scaleTargetRef != () {
                    doc.spec.scaleTargetRef.name.replace(name, "{{instance.appslug}}");
                }
                if doc.kind == "Ingress" {
                    doc = replace_ingress(doc, name);
                }
                if doc.kind == "Certificate" && "secretName" in doc.spec && doc.spec.secretName != () {
                    doc.spec.secretName.replace(name, "{{instance.appslug}}");
                }

                // Selector/label replacement for service-related resources
                let res_comp = "";
                if doc.kind == "Service" && doc_name in analysis.svc_comp_map {
                    res_comp = analysis.svc_comp_map[doc_name];
                    doc = apply_svc_selectors(doc, res_comp);
                } else if doc.kind == "PodDisruptionBudget" && "spec" in doc && doc.spec != () && "selector" in doc.spec && doc.spec.selector != () && "matchLabels" in doc.spec.selector && doc.spec.selector.matchLabels != () {
                    res_comp = find_comp_for_selector(analysis, doc.spec.selector.matchLabels);
                    if res_comp != "" {
                        doc.spec.selector.matchLabels = `SELECTOR_COMP_${res_comp}`;
                    }
                } else if doc.kind == "NetworkPolicy" && "spec" in doc && doc.spec != () && "podSelector" in doc.spec && doc.spec.podSelector != () && "matchLabels" in doc.spec.podSelector && doc.spec.podSelector.matchLabels != () {
                    res_comp = find_comp_for_selector(analysis, doc.spec.podSelector.matchLabels);
                    if res_comp != "" {
                        doc.spec.podSelector.matchLabels = `SELECTOR_COMP_${res_comp}`;
                    }
                } else if doc.kind == "ServiceMonitor" && "spec" in doc && doc.spec != () && "selector" in doc.spec && doc.spec.selector != () && "matchLabels" in doc.spec.selector && doc.spec.selector.matchLabels != () {
                    res_comp = find_comp_for_svc_labels(analysis, doc.spec.selector.matchLabels);
                    if res_comp != "" {
                        doc.spec.selector.matchLabels = `SELECTOR_COMP_${res_comp}`;
                    }
                }

                if doc.kind == "ServiceMonitor" {
                    let fp = `${path}/get_others/${doc.kind}_${file_name}.yaml.hbs`;
                    gen_yaml_if(fp, "servicemonitors.monitoring.coreos.com", doc);
                    if res_comp != "" { apply_selector_expressions(fp, res_comp); }
                } else if doc.kind == "PrometheusRule" {
                    gen_yaml_if(`${path}/get_others/${doc.kind}_${file_name}.yaml.hbs`, "prometheusrules.monitoring.coreos.com", doc);
                } else if doc.kind == "Certificate" {
                    gen_yaml_if(`${path}/get_others/${doc.kind}_${file_name}.yaml.hbs`, "certificates.cert-manager.io", doc);
                } else {
                    let fp = `${path}/get_others/${doc.kind}_${file_name}.yaml.hbs`;
                    gen_yaml(fp, doc);
                    if img_extr.images.len() > 0 || img_extr.resources.len() > 0 {
                        apply_image_resource_expressions(fp, img_extr.images.keys(), img_extr.resources.keys());
                    }
                    if res_comp != "" { apply_selector_expressions(fp, res_comp); }
                }
           }
        }
        update_package_yaml_images_resources(path, all_images, all_resources);
    }
}
fn gen_tenant(path, docs, name) {
    gen_tenant_core(path, docs, name, ());
}
fn gen_tenant(path, docs) {
    let yaml = core::yaml_decode(host::file_read(path+"/package.yaml"));
    let name = if type_of(yaml["metadata"]) == "map" {yaml["metadata"]["name"]} else {()};
    gen_tenant_core(path, docs, name, ());
    apply_dedup_to_generated(path, name);
}
fn gen_tenant(path, docs, name, ns) {
    gen_tenant_core(path, docs, name, ns);
    apply_name_to_generated(path, name);
    apply_namespace_to_generated(path, ns);
    apply_dedup_to_generated(path, read_pkg_name(path));
}

// ---------------------------------------------------------------------------
// gen_service
// ---------------------------------------------------------------------------

fn gen_service_core(path, docs, name, ns) {
    if name != () {
        let analysis = build_analysis(docs, name);
        let all_images = #{};
        let all_resources = #{};
        for doc in docs.filter(|obj| type_of(obj) == "map" && obj.keys().contains("kind") && obj.keys().contains("metadata") && type_of(obj.metadata) == "map" && obj.kind != ()) {
            let doc_name = doc.metadata.name;
            let file_name = clean_file_name(`${doc_name}`, name, ns);
            doc.metadata.name = replace_first(`${doc.metadata.name}`, name, "{{instance.appslug}}");
            if "namespace" in doc.metadata {
                doc.metadata.remove("namespace");
            }
            doc.metadata = clean_metadata(doc.metadata, name);
            if ["PersistentVolumeClaim"].contains(doc.kind) {
                host::create_dir(`${path}/get_vitals`);
                gen_yaml(`${path}/get_vitals/${doc.kind}_${file_name}.yaml.hbs`, doc);
            } else if ["Deployment", "ReplicaSet", "StatefulSet", "DaemonSet"].contains(doc.kind) {
                host::create_dir(`${path}/get_scalables`);
                let extr = extract_workload_images_resources(doc, name);
                doc = extr.doc;
                for k in extr.images.keys() { all_images[k] = extr.images[k]; }
                for k in extr.resources.keys() { all_resources[k] = extr.resources[k]; }
                doc = replace_workload(path, doc_name, doc, name);
                let comp = if doc_name in analysis.comp_map { analysis.comp_map[doc_name] } else { "" };
                if comp != "" { doc = apply_workload_selectors(doc, comp); }
                let fp = `${path}/get_scalables/${doc.kind}_${file_name}.yaml.hbs`;
                gen_yaml(fp, doc);
                if extr.images.len() > 0 || extr.resources.len() > 0 {
                    apply_image_resource_expressions(fp, extr.images.keys(), extr.resources.keys());
                }
                if comp != "" { apply_selector_expressions(fp, comp); }
            } else if doc.kind == "CustomResourceDefinition" {
                host::create_dir(`${path}/get_crds`);
                doc.metadata = clean_metadata(doc.metadata, name);
                if "conversion" in doc.spec && doc.spec.conversion != () {
                    if "webhook" in doc.spec.conversion && doc.spec.conversion.webhook != () {
                        doc.spec.conversion.webhook.clientConfig.service.name.replace(name, "{{instance.appslug}}");
                        doc.spec.conversion.webhook.clientConfig.service.namespace = "{{instance.namespace}}";
                    }
                    gen_crd_yaml(`${path}/get_crds/${doc.metadata.name}.yaml.hbs`, doc);
                } else {
                    gen_crd_yaml(`${path}/get_crds/${doc.metadata.name}.yaml`, doc);
                }
            } else if ["Namespace"].contains(doc.kind) {
                host::create_dir(`${path}/get_systems`);
                gen_yaml(`${path}/get_systems/${doc.kind}_${file_name}.yaml.hbs`, doc);
            } else {
                host::create_dir(`${path}/get_others`);
                if doc.kind == "RoleBinding" || doc.kind == "ClusterRoleBinding" {
                    doc = replace_binding(doc, name);
                }
                if doc.kind == "ValidatingWebhookConfiguration" || doc.kind == "MutatingWebhookConfiguration" {
                    doc = replace_webhooks(doc, name);
                }
                if doc.kind == "APIService" {
                    doc.spec.service.name.replace(name, "{{instance.appslug}}");
                    doc.spec.service.namespace = "{{instance.namespace}}";
                }
                let img_extr = #{images: #{}, resources: #{}};
                if doc.kind == "Job" {
                    img_extr = extract_workload_images_resources(doc, name);
                    doc = img_extr.doc;
                    for k in img_extr.images.keys() { all_images[k] = img_extr.images[k]; }
                    for k in img_extr.resources.keys() { all_resources[k] = img_extr.resources[k]; }
                    doc = replace_job(path, doc_name, doc, name);
                }
                if doc.kind == "CronJob" {
                    img_extr = extract_cronjob_images_resources(doc, name);
                    doc = img_extr.doc;
                    for k in img_extr.images.keys() { all_images[k] = img_extr.images[k]; }
                    for k in img_extr.resources.keys() { all_resources[k] = img_extr.resources[k]; }
                    doc = replace_cronjob(path, doc_name, doc, name);
                }
                if doc.kind == "HorizontalPodAutoscaler" && "scaleTargetRef" in doc.spec && doc.spec.scaleTargetRef != () {
                    doc.spec.scaleTargetRef.name.replace(name, "{{instance.appslug}}");
                }
                if doc.kind == "Ingress" {
                    doc = replace_ingress(doc, name);
                }
                if doc.kind == "Certificate" && "secretName" in doc.spec && doc.spec.secretName != () {
                    doc.spec.secretName.replace(name, "{{instance.appslug}}");
                }

                // Selector/label replacement for service-related resources
                let res_comp = "";
                if doc.kind == "Service" && doc_name in analysis.svc_comp_map {
                    res_comp = analysis.svc_comp_map[doc_name];
                    doc = apply_svc_selectors(doc, res_comp);
                } else if doc.kind == "PodDisruptionBudget" && "spec" in doc && doc.spec != () && "selector" in doc.spec && doc.spec.selector != () && "matchLabels" in doc.spec.selector && doc.spec.selector.matchLabels != () {
                    res_comp = find_comp_for_selector(analysis, doc.spec.selector.matchLabels);
                    if res_comp != "" {
                        doc.spec.selector.matchLabels = `SELECTOR_COMP_${res_comp}`;
                    }
                } else if doc.kind == "NetworkPolicy" && "spec" in doc && doc.spec != () && "podSelector" in doc.spec && doc.spec.podSelector != () && "matchLabels" in doc.spec.podSelector && doc.spec.podSelector.matchLabels != () {
                    res_comp = find_comp_for_selector(analysis, doc.spec.podSelector.matchLabels);
                    if res_comp != "" {
                        doc.spec.podSelector.matchLabels = `SELECTOR_COMP_${res_comp}`;
                    }
                } else if doc.kind == "ServiceMonitor" && "spec" in doc && doc.spec != () && "selector" in doc.spec && doc.spec.selector != () && "matchLabels" in doc.spec.selector && doc.spec.selector.matchLabels != () {
                    res_comp = find_comp_for_svc_labels(analysis, doc.spec.selector.matchLabels);
                    if res_comp != "" {
                        doc.spec.selector.matchLabels = `SELECTOR_COMP_${res_comp}`;
                    }
                }

                if doc.kind == "ServiceMonitor" {
                    let fp = `${path}/get_others/${doc.kind}_${file_name}.yaml.hbs`;
                    gen_yaml_if(fp, "servicemonitors.monitoring.coreos.com", doc);
                    if res_comp != "" { apply_selector_expressions(fp, res_comp); }
                } else if doc.kind == "PrometheusRule" {
                    gen_yaml_if(`${path}/get_others/${doc.kind}_${file_name}.yaml.hbs`, "prometheusrules.monitoring.coreos.com", doc);
                } else if doc.kind == "Certificate" {
                    gen_yaml_if(`${path}/get_others/${doc.kind}_${file_name}.yaml.hbs`, "certificates.cert-manager.io", doc);
                } else {
                    let fp = `${path}/get_others/${doc.kind}_${file_name}.yaml.hbs`;
                    gen_yaml(fp, doc);
                    if img_extr.images.len() > 0 || img_extr.resources.len() > 0 {
                        apply_image_resource_expressions(fp, img_extr.images.keys(), img_extr.resources.keys());
                    }
                    if res_comp != "" { apply_selector_expressions(fp, res_comp); }
                }
           }
        }
        update_package_yaml_images_resources(path, all_images, all_resources);
    }
}
fn gen_service(path, docs, name) {
    gen_service_core(path, docs, name, ());
}
fn gen_service(path, docs) {
    let yaml = core::yaml_decode(host::file_read(path+"/package.yaml"));
    let name = if type_of(yaml["metadata"]) == "map" {yaml["metadata"]["name"]} else {()};
    gen_service_core(path, docs, name, ());
    apply_dedup_to_generated(path, name);
}
fn gen_service(path, docs, name, ns) {
    gen_service_core(path, docs, name, ns);
    apply_name_to_generated(path, name);
    apply_namespace_to_generated(path, ns);
    apply_dedup_to_generated(path, read_pkg_name(path));
}
