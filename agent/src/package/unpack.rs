use clap::Args;
use common::{Error, Result, ocihandler::Registry, rhaihandler::base64_decode};
use std::path::{Path, PathBuf};
/*
#[allow(non_snake_case)]
#[derive(Deserialize, Serialize, Clone, Debug)]
struct Layer {
    digest: String,
    mediaType: String,
    size: i64,
}*/

#[derive(Args, Debug)]
pub struct Parameters {
    /// Destination directory
    #[arg(
        short = 'd',
        long = "destination",
        env = "PACKAGE_DIRECTORY",
        value_name = "PACKAGE_DIRECTORY",
        default_value = "/tmp/package"
    )]
    destination: PathBuf,
    /// Registry
    #[arg(
        short = 'r',
        long = "registry",
        env = "REGISTRY",
        value_name = "REGISTRY",
        default_value = "oci.solidite.fr"
    )]
    registry: String,
    /// Image
    #[arg(short = 'i', long = "image", env = "IMAGE", value_name = "IMAGE")]
    image: String,
    /// Tag
    #[arg(
        short = 't',
        long = "tag",
        env = "TAG",
        value_name = "TAG",
        default_value = "1.0.0"
    )]
    tag: String,
    /// Username
    #[arg(
        short = 'u',
        long = "username",
        env = "USERNAME",
        value_name = "USERNAME",
        default_value = ""
    )]
    username: String,
    /// Password
    #[arg(
        short = 'p',
        long = "password",
        env = "PASSWORD",
        value_name = "PASSWORD",
        default_value = ""
    )]
    password: String,
    /// pull-secret mount-path
    #[arg(
        long = "pull-secret-path",
        env = "PULL_SECRET_PATH",
        value_name = "PULL_SECRET_PATH",
        default_value = ""
    )]
    pull_path: String,
}

pub fn run(args: &Parameters) -> Result<()> {
    if Path::new(&args.destination).is_dir() {
        let mut cli = if args.pull_path.is_empty() {
            Registry::new(
                args.registry.clone(),
                args.username.clone(),
                args.password.clone(),
            )
        } else {
            let pull_secret_string = std::fs::read_to_string(format!("{}/.dockerconfigjson", args.pull_path))
                .map_err(Error::Stdio)?;
            let pull_secret: serde_json::Value =
                serde_json::from_str(&pull_secret_string).map_err(Error::SerializationError)?;
            let hash = pull_secret
                .get("auths")
                .and_then(|a| a.get(args.registry.as_str()))
                .and_then(|r| r.get("auth"))
                .cloned()
                .unwrap_or_default();
            let hash_str = hash.as_str().unwrap_or_default();
            let user_pass = base64_decode(hash_str)?;
            let auth: Vec<&str> = user_pass.split(':').collect();
            let user = auth.first().copied().unwrap_or_default().to_string();
            let pass = auth.get(1).copied().unwrap_or_default().to_string();
            Registry::new(args.registry.clone(), user, pass)
        };
        cli.pull_image(&args.destination, args.image.clone(), args.tag.clone())?;
        Ok(())
    } else {
        tracing::error!("{:?} is not a directory", &args.destination);
        Err(Error::MissingDestination(args.destination.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, Parameters, run};
    use std::path::PathBuf;

    /// Scénario « auth base64 malformé » : un secret monté dont `auths.<registry>.auth`
    /// n'est pas du base64 valide doit renvoyer une erreur `Core` sans panic, avant tout
    /// contact registre (le décodage précède `Registry::pull_image`).
    #[test]
    fn auth_base64_malformé() {
        let destination = tempfile::tempdir().expect("tempdir destination");
        let secret_dir = tempfile::tempdir().expect("tempdir secret");
        let secret = r#"{"auths":{"oci.solidite.fr":{"auth":"!!!pas-du-base64!!!"}}}"#;
        std::fs::write(secret_dir.path().join(".dockerconfigjson"), secret).expect("write .dockerconfigjson");
        let args = Parameters {
            destination: PathBuf::from(destination.path()),
            registry: "oci.solidite.fr".to_string(),
            image: "my-app".to_string(),
            tag: "1.0.0".to_string(),
            username: String::new(),
            password: String::new(),
            pull_path: secret_dir.path().display().to_string(),
        };
        let result = run(&args);
        assert!(
            matches!(result, Err(Error::Core(_))),
            "attendu Err(Core), obtenu {result:?}"
        );
    }
}
