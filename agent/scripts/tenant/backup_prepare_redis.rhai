fn run(context) {
    import_run("backup_prepare_redis_pre", context);
    for redis in context.redis_list {
        host::log_info(`Dumping redis target: ${redis}`);
        let ns  = context.namespace;
        let sts = host::get_env(`${redis}_sts`);
        let pass = host::get_env(`${redis}_password`);
        let pod = host::shell_output(`kubectl -n ${ns} get pods -l app=${sts} --field-selector=status.phase=Running -o jsonpath='{.items[0].metadata.name}'`);
        if pod == "" {
            throw `No running pod found for ${redis} (sts: ${sts})`;
        }
        let rc = host::shell_run(`kubectl -n ${ns} exec ${pod} -- redis-cli --no-auth-warning -a "${pass}" SAVE`);
        if rc != 0 {
            throw `redis SAVE failed for ${redis}`;
        }
        let rc = host::shell_run(`kubectl -n ${ns} cp ${pod}:/data/dump.rdb /backup/redis_${redis}.rdb`);
        if rc != 0 {
            throw `kubectl cp failed for ${redis}`;
        }
    }
    import_run("backup_prepare_redis_post", context);
}
