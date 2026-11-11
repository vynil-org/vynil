// create a run context for backup/restore pod in from the preparation of the "run" function bellow
fn from_args(context) {
    context["deployment_list"] = host::get_env("DEPLOYMENT_LIST").split(" ").filter(|x| x!="");
    context["statefulset_list"] = host::get_env("STATEFULSET_LIST").split(" ").filter(|x| x!="");
    context["secret_list"] = host::get_env("SECRET_LIST").split(" ").filter(|x| x!="");
    context["pg_list"] = host::get_env("PG_LIST").split(" ").filter(|x| x!="");
    context["mysql_list"] = host::get_env("MYSQL_LIST").split(" ").filter(|x| x!="");
    context["mongo_list"] = host::get_env("MONGO_LIST").split(" ").filter(|x| x!="");
    context["redis_list"] = host::get_env("REDIS_LIST").split(" ").filter(|x| x!="");
    context["restic_tags"] = host::get_env("RESTIC_TAGS");
    context["snapshot"] = host::get_env("RESTIC_SNAPSHOT");
    context["max_dayly"] = host::get_env("RESTIC_MAX_DAYLY");
    if context["max_dayly"] == "" {context["max_dayly"] = "7";}
    context["max_weekly"] = host::get_env("RESTIC_MAX_WEEKLY");
    if context["max_weekly"] == "" {context["max_weekly"] = "5";}
    context["max_monthly"] = host::get_env("RESTIC_MAX_MONTHLY");
    if context["max_monthly"] == "" {context["max_monthly"] = "12";}
    context["max_yearly"] = host::get_env("RESTIC_MAX_YEARLY");
    if context["max_yearly"] == "" {context["max_yearly"] = "4";}
    context["namespace"] = host::get_env("NAMESPACE");
    try {
        context["scale_target"] = parse_int(host::get_env("SCALE_TARGET"));
    } catch {
        context["scale_target"] = 1;
    }
    let sub_path  = host::get_env("SUB_PATH");
    let base_path = host::get_env("BASE_REPO_URL");
    sub_path.replace("/"," ");
    sub_path.trim();
    sub_path.replace(" ","/");
    base_path.replace("/"," ");
    base_path.trim();
    base_path.replace(" ","/");
    context["sub_path"] = sub_path;
    context["base_path"] = base_path;
    context["s3_url"] = `s3:${base_path}/${sub_path}`;
    context["restic_args"] = "";
    if host::get_env("INSECURE_TLS") == "true" {
        context["restic_args"] += " --insecure-tls";
    }
    context
}

fn run(instance, context, use_init_from) {
    let secret_name = "backup-settings";
    let sub_path = `${context.instance.namespace}/${context.instance.appslug}`;
    if use_init_from {
        if instance.spec.initFrom.secretName != () {
            secret_name = instance.spec.initFrom.secretName;
        }
        if instance.spec.initFrom.subPath != () {
            sub_path = instance.spec.initFrom.subPath;
        }
    }
    context["volumes"] = [#{
        name: "empty-dir",
        emptyDir: #{}
    }];
    context["mounts"] = [#{
        name: "empty-dir",
        mountPath: `/backup`
    }];
    context["envs_from"] = [#{
        secretRef: #{
            name: secret_name
        }
    }];
    let restic_tags = `namespace:${context.instance.namespace},instance:${context.instance.name},component:${context.instance["package"].name},version:${context.instance.requested}`;
    if context.instance["package"].type == "tenant" {
        restic_tags += `,tenant:${context.tenant.name}`;
    }
    context["envs"] = [#{
        name: "RESTIC_TAGS",
        value: restic_tags
    }, #{
        name: "RESTIC_HOST",
        value: context.instance.appslug
    }, #{
        name: "GOMAXPROCS",
        value: "2"
    }, #{
        name: "GOMEMLIMIT",
        value: "1560MiB"
    }, #{
        name: "GOGC",
        value: "50"
    }, #{
        name: "AGENT_IMAGE",
        value: context.agent_image
    }, #{
        name: "SCALE_TARGET",
        value: if context.namespace.ha { "2" } else { "1" }
    }, #{
        name: "VYNIL_NAMESPACE",
        value: context.cluster.vynil_namespace
    }, #{
        name: "INSTANCE",
        value: context.instance.name
    }, #{
        name: "TAG",
        value: context.instance.requested
    }, #{
        name: "SUB_PATH",
        value: sub_path
    }, #{
        name: "APPSLUG",
        value: context.instance.appslug
    }, #{
        name: "NAMESPACE",
        valueFrom: #{
            fieldRef: #{
                apiVersion: "v1",
                fieldPath: "metadata.namespace"
            }
        }
    }, #{
        name: "POD_NAME",
        valueFrom: #{
            fieldRef: #{
                apiVersion: "v1",
                fieldPath: "metadata.name"
            }
        }
    }];
    if use_init_from {
        context["envs"] += #{
            name: "RESTIC_SNAPSHOT",
            value: instance.spec.initFrom.snapshot
        };
    }
    context["files"] = [];
    if host::is_dir(`${context.package_dir}/scripts`) {
        for f in host::read_dir(`${context.package_dir}/scripts`) {
            let base = core::basename(f);
            if base.starts_with("restore") || base.starts_with("backup") || base.starts_with("maintenance") {
                context["files"] += #{
                    name: base,
                    content: host::file_read(f)
                };
            }
        }
    }
    context["has_files"] = context["files"].len() > 0;
    if context["has_files"] {
        context["volumes"] += #{
            name: "backups-scripts",
            configMap: #{
                defaultMode: 493,
                name: `${context.instance.appslug}-backups-scripts`
            }
        };
        context["mounts"] += #{
            name: "backups-scripts",
            mountPath: "/package/scripts"
        };
    }
    context["schedule"] = `${context.namespace.maintenance_start_minut} ${context.namespace.maintenance_start_hour} * * *`;
    context["service_account"] = `${context.instance.appslug}-backup`;
    context["backup_affinity"] = context.instance["package"].backup_affinity;
    let pgs = [];
    let mysqls = [];
    let mongos = [];
    let redises = [];
    let secrets = [];
    for v in instance.status.vitals {
        let name = v.name;
        name.replace(context.instance.appslug,"");
        name.replace("-"," ");
        name.trim();
        name.replace(" ","-");
        if v.kind == "PersistentVolumeClaim" {
            if name.is_empty() {
                name = "data";
            }
            context["volumes"] += #{
                name: name,
                persistentVolumeClaim: #{
                    claimName: v.name
                }
            };
            context["mounts"] += #{
                name: name,
                mountPath: `/backup/${name}`
            };
        } else if v.kind == "Secret" {
            if name.is_empty() {
                name = "secret";
            }
            secrets += name;
            if ! use_init_from {
                context["volumes"] += #{
                    name: name,
                    secret: #{
                        secretName: v.name
                    }
                };
                context["mounts"] += #{
                    name: name,
                    mountPath: `/secrets/${name}`
                };
            }
            context["envs"] += #{
                name: `secret_${name}_target`,
                value: v.name
            };
        } else if v.kind == "Cluster" {
            if name.is_empty() {
                name = "postgres";
            }
            pgs += name;
            let lst = k8s_resource("Secret", context.instance.namespace).list_meta().items;
            let secret = `${v.name}-app`;
            if lst.filter(|s| s.metadata.name == `${v.name}-superuser`).len() > 0 {
                secret = `${v.name}-superuser`;
            }
            for i in ["host", "username", "password", "dbname"] {
                context["envs"] += #{
                    name: `${name}_${i}`,
                    valueFrom: #{
                        secretKeyRef: #{
                            name: secret,
                            key: i
                        }
                    }
                };
            }
        } else if v.kind == "MariaDB" {
            if name.is_empty() {
                name = "mariadb";
            }
            try {
                let mariadb = k8s_resource("MariaDB", context.instance.namespace).get(v.name);
                let sname = mariadb.spec?.rootPasswordSecretKeyRef?.name;
                let username = mariadb.spec?.username;
                let upass = mariadb.spec?.passwordSecretKeyRef?.name;
                let dbname = mariadb.spec?.database;
                if sname != () {
                    mysqls += name;
                    context["envs"] += #{
                        name: `${name}_username`,
                        value: "root"
                    };
                    context["envs"] += #{
                        name: `${name}_password`,
                        valueFrom: #{
                            secretKeyRef: #{
                                name: sname,
                                key: mariadb.spec?.rootPasswordSecretKeyRef?.key ?? "password"
                            }
                        }
                    };
                    context["envs"] += #{
                        name: `${name}_host`,
                        value: `${v.name}.${context.instance.namespace}.svc`
                    };
                    context["envs"] += #{
                        name: `${name}_dbname`,
                        value: dbname ?? "*"
                    };
                } else if upass != () && username != () && dbname != () {
                    mysqls += name;
                    context["envs"] += #{
                        name: `${name}_username`,
                        value: username
                    };
                    context["envs"] += #{
                        name: `${name}_password`,
                        valueFrom: #{
                            secretKeyRef: #{
                                name: upass,
                                key: mariadb.spec?.passwordSecretKeyRef?.key ?? "password"
                            }
                        }
                    };
                    context["envs"] += #{
                        name: `${name}_host`,
                        value: `${v.name}.${context.instance.namespace}.svc`
                    };
                    context["envs"] += #{
                        name: `${name}_dbname`,
                        value: dbname
                    };
                }
            } catch (e) {
                switch type_of(e) {
                    "string" => host::log_warn(e),
                    _ => host::log_warn(core::json_encode(e))
                }
            }
        } else if v.kind == "NdbCluster" {
            if name.is_empty() {
                name = "ndb";
            }
            try {
                let ndb = k8s_resource("NdbCluster", context.instance.namespace).get(v.name);
                let sname = ndb.spec?.mysqlNode?.rootPasswordSecretName;
                if sname != () {
                    mysqls += name;
                    for i in ["username", "password"] {
                        context["envs"] += #{
                            name: `${name}_${i}`,
                            valueFrom: #{
                                secretKeyRef: #{
                                    name: sname,
                                    key: i
                                }
                            }
                        };
                    }
                    context["envs"] += #{
                        name: `${name}_host`,
                        value: `${v.name}-mysqld.${context.instance.namespace}.svc`
                    };
                    context["envs"] += #{
                        name: `${name}_dbname`,
                        value: "*"
                    };
                }
            } catch (e) {
                switch type_of(e) {
                    "string" => host::log_warn(e),
                    _ => host::log_warn(core::json_encode(e))
                }
            }
        } else if v.kind == "MongoDBCommunity" {
            if name.is_empty() {
                name = "mongodb";
            }
            try {
                let mongodb = k8s_resource("MongoDBCommunity", context.instance.namespace).get(v.name);
                let users = mongodb.spec?.users;
                if users != () && users.len() > 0 {
                    let conn_secret = users[0].connectionStringSecretName;
                    mongos += name;
                    context["envs"] += #{
                        name: `${name}_username`,
                        valueFrom: #{
                            secretKeyRef: #{
                                name: conn_secret,
                                key: "username"
                            }
                        }
                    };
                    context["envs"] += #{
                        name: `${name}_password`,
                        valueFrom: #{
                            secretKeyRef: #{
                                name: conn_secret,
                                key: "password"
                            }
                        }
                    };
                    context["envs"] += #{
                        name: `${name}_host`,
                        value: `${v.name}-svc.${context.instance.namespace}.svc`
                    };
                    context["envs"] += #{
                        name: `${name}_dbname`,
                        value: "*"
                    };
                }
            } catch (e) {
                switch type_of(e) {
                    "string" => host::log_warn(e),
                    _ => host::log_warn(core::json_encode(e))
                }
            }
        } else if v.kind == "Dragonfly" {
            if name.is_empty() {
                name = "dragonfly";
            }
            try {
                let df = k8s_resource("Dragonfly", context.instance.namespace).get(v.name);
                let sname = df.spec?.authentication?.passwordFromSecret?.name;
                let skey  = df.spec?.authentication?.passwordFromSecret?.key ?? "password";
                if sname != () {
                    redises += name;
                    context["envs"] += #{
                        name: `${name}_password`,
                        valueFrom: #{
                            secretKeyRef: #{
                                name: sname,
                                key: skey
                            }
                        }
                    };
                    context["envs"] += #{
                        name: `${name}_host`,
                        value: `${v.name}.${context.instance.namespace}.svc`
                    };
                    context["envs"] += #{
                        name: `${name}_kind`,
                        value: "Dragonfly"
                    };
                    context["envs"] += #{
                        name: `${name}_sts`,
                        value: v.name
                    };
                }
            } catch (e) {
                switch type_of(e) {
                    "string" => host::log_warn(e),
                    _ => host::log_warn(core::json_encode(e))
                }
            }
        } else if v.kind == "RedisCluster" {
            if name.is_empty() {
                name = "redis";
            }
            try {
                let rc = k8s_resource("RedisCluster", context.instance.namespace).get(v.name);
                let sname = rc.spec?.kubernetesConfig?.redisSecret?.name;
                let skey  = rc.spec?.kubernetesConfig?.redisSecret?.key ?? "password";
                if sname != () {
                    redises += name;
                    context["envs"] += #{
                        name: `${name}_password`,
                        valueFrom: #{
                            secretKeyRef: #{
                                name: sname,
                                key: skey
                            }
                        }
                    };
                    context["envs"] += #{
                        name: `${name}_host`,
                        value: `${v.name}-leader.${context.instance.namespace}.svc`
                    };
                    context["envs"] += #{
                        name: `${name}_kind`,
                        value: "RedisCluster"
                    };
                    context["envs"] += #{
                        name: `${name}_sts`,
                        value: `${v.name}-leader`
                    };
                }
            } catch (e) {
                switch type_of(e) {
                    "string" => host::log_warn(e),
                    _ => host::log_warn(core::json_encode(e))
                }
            }
        } else if v.kind == "RedisSentinel" {
            if name.is_empty() {
                name = "redis";
            }
            try {
                let rs = k8s_resource("RedisSentinel", context.instance.namespace).get(v.name);
                let sname = rs.spec?.kubernetesConfig?.redisSecret?.name;
                let skey  = rs.spec?.kubernetesConfig?.redisSecret?.key ?? "password";
                if sname != () {
                    redises += name;
                    context["envs"] += #{
                        name: `${name}_password`,
                        valueFrom: #{
                            secretKeyRef: #{
                                name: sname,
                                key: skey
                            }
                        }
                    };
                    context["envs"] += #{
                        name: `${name}_host`,
                        value: `${v.name}-master.${context.instance.namespace}.svc`
                    };
                    context["envs"] += #{
                        name: `${name}_kind`,
                        value: "RedisSentinel"
                    };
                    context["envs"] += #{
                        name: `${name}_sts`,
                        value: `${v.name}-master`
                    };
                }
            } catch (e) {
                switch type_of(e) {
                    "string" => host::log_warn(e),
                    _ => host::log_warn(core::json_encode(e))
                }
            }
        }
    }
    let deploy = [];
    let sts = [];
    if instance.status.scalables != () {
        for s in instance.status.scalables {
            if s.kind == "Deployment" {
                deploy += s.name;
            } else if s.kind == "StatefulSet" {
                sts += s.name;
            }
        }
    }
    if deploy.len() > 0 {
        context["envs"] += #{
            name: `DEPLOYMENT_LIST`,
            value: deploy.reduce(|sum, v| if sum.type_of() == "()" { v } else { `${sum} ${v}` })
        };
    }
    if sts.len() > 0 {
        context["envs"] += #{
            name: `STATEFULSET_LIST`,
            value: sts.reduce(|sum, v| if sum.type_of() == "()" { v } else { `${sum} ${v}` })
        };
    }
    if pgs.len() > 0 {
        context["envs"] += #{
            name: `PG_LIST`,
            value: pgs.reduce(|sum, v| if sum.type_of() == "()" { v } else { `${sum} ${v}` })
        };
    }
    if mysqls.len() > 0 {
        context["envs"] += #{
            name: `MYSQL_LIST`,
            value: mysqls.reduce(|sum, v| if sum.type_of() == "()" { v } else { `${sum} ${v}` })
        };
    }
    if mongos.len() > 0 {
        context["envs"] += #{
            name: `MONGO_LIST`,
            value: mongos.reduce(|sum, v| if sum.type_of() == "()" { v } else { `${sum} ${v}` })
        };
    }
    if redises.len() > 0 {
        context["envs"] += #{
            name: `REDIS_LIST`,
            value: redises.reduce(|sum, v| if sum.type_of() == "()" { v } else { `${sum} ${v}` })
        };
    }
    if secrets.len() > 0 {
        context["envs"] += #{
            name: `SECRET_LIST`,
            value: secrets.reduce(|sum, v| if sum.type_of() == "()" { v } else { `${sum} ${v}` })
        };
    }
    context
}
