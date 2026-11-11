fn resolv_cm(svc) {
    if svc?.definition?.kind == "ConfigMap" {
        try {
            let cfg = k8s_resource("ConfigMap", svc.definition.namespace).get(svc.definition.name);
            svc += cfg.data;
        } catch (e) {
            switch type_of(e) {
                "string" => host::log_warn(e),
                _ => host::log_warn(core::json_encode(e))
            }
        }
        svc.remove("definition");
    }
    svc
}

fn get_from_key(svcs, names) {
    if svcs == () {
        return ();
    }
    if type_of(names) == "string" {
        let found = svcs.filter(|s| s.key == names);
        if found.len() > 0 {
            return resolv_cm(found[0]);
        } else {
            return ();
        }
    }
    for n in names {
        let found = svcs.filter(|s| s.key == n);
        if found.len() > 0 {
            return resolv_cm(found[0]);
        }
    }
    ()
}

fn service_glob(svcs, matcher) {
    if svcs == () {
        return ();
    }
    let found = svcs.filter(|s| s["package"].name.glob(matcher));
    if found.len() > 0 {
        return found.map(|s| resolv_cm(s));
    } else {
        return ();
    }
    ()
}

fn get_from_package(svcs, names) {
    if svcs == () {
        return ();
    }
    if type_of(names) == "string" {
        let found = svcs.filter(|s| s["package"].name == names);
        if found.len() > 0 {
            return resolv_cm(found[0]);
        } else {
            return ();
        }
    }
    for n in names {
        let found = svcs.filter(|s| s["package"].name == n);
        if found.len() > 0 {
            return resolv_cm(found[0]);
        }
    }
    ()
}

fn get_from_package(svcs, key, names) {
    if svcs == () {
        return ();
    }
    if type_of(names) == "string" {
        let found = svcs.filter(|s| s.key == key && s["package"].name == names);
        if found.len() > 0 {
            return resolv_cm(found[0]);
        } else {
            return ();
        }
    }
    for n in names {
        let found = svcs.filter(|s| s.key == key && s["package"].name == n);
        if found.len() > 0 {
            return resolv_cm(found[0]);
        }
    }
    ()
}

fn get_system(context, names) {
    get_from_key(context?.cluster?.services, names)
}

fn get_tenant(context, names) {
    get_from_key(context?.tenant?.services, names)
}

fn get_system_package(context, names) {
    get_from_package(context?.cluster?.services, names)
}

fn get_tenant_package(context, names) {
    get_from_package(context?.tenant?.services, names)
}

fn get_system_package(context, key, names) {
    get_from_package(context?.cluster?.services, key, names)
}

fn get_tenant_package(context, key, names) {
    get_from_package(context?.tenant?.services, key, names)
}

fn glob_system_package(context, pattern) {
    service_glob(context?.cluster?.services, pattern)
}

fn glob_tenant_package(context, pattern) {
    service_glob(context?.tenant?.services, pattern)
}

fn resolv_secret(svc) {
    if svc?.definition?.kind == "Secret" {
        try {
            let data = k8s_resource("Secret", svc.definition.namespace).get(svc.definition.name).data;
            for k in data.keys() {
                svc[k] = core::base64_decode(data[k]);
            }
        } catch (e) {
            switch type_of(e) {
                "string" => host::log_warn(e),
                _ => host::log_warn(core::json_encode(e))
            }
        }
        svc.remove("definition");
    }
    svc
}
