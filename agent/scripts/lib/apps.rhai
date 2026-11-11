fn get_authentik(ns, name) {
    let auth = k8s_resource("Secret", ns).get(name);
    let cert = k8s_resource("Secret", ns).get(name+"-cert");
    let ing = k8s_resource("Ingress", ns).get(name);
    let res = new_http_client("https://"+ing.spec.rules[0].host+"/api/v3");
    res.add_header_json();
    res.add_header_bearer(core::base64_decode(auth.data.AUTHENTIK_BOOTSTRAP_TOKEN));
    if ("ca.crt" in cert.data.keys()) {
        res.set_server_ca(core::base64_decode(cert.data["ca.crt"]));
    }
    res
}
fn get_authentik(ns) {
    get_authentik(ns,"authentik")
}

fn get_sonar(ns,name) {
    let auth = k8s_resource("Secret", ns).get(name+"-sonar");
    let res = new_http_client("http://"+name+"-sonar."+ns+".svc/api");
    res.add_header_json();
    res.add_header_basic("admin", core::base64_decode(auth.data.ADMIN_PASSWORD));
    res
}
fn get_sonar(ns) {
    get_sonar(ns, "sonar")
}

fn get_gitea(ns, name) {
    let auth = k8s_resource("Secret", ns).get(name+"-admin-user");
    let res = new_http_client("http://"+name+"-http."+ns+".svc:3000/api/v1");
    res.add_header_json();
    res.add_header_basic(core::base64_decode(auth.data.username), core::base64_decode(auth.data.password));
    res
}
fn get_gitea(ns) {
    get_gitea(ns, "gitea")
}
fn gitea_list_orgs(gitea) {
    gitea.get("orgs")
}
