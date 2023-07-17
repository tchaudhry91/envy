use std::path::Path;

use anyhow::Result;
use handlebars::Handlebars;
use log::debug;
use log::log_enabled;
use serde::Deserialize;
use serde::Serialize;
use subprocess::{Popen, PopenConfig};

use super::templates::{DOCKER_IGNORE, TEMPLATE_DOCKERFILE};

use super::package::{PType, Pack};
use super::utils;

pub fn check_docker() -> Result<()> {
    let mut p = Popen::create(
        &["docker", "ps"],
        PopenConfig {
            stdout: subprocess::Redirection::Pipe,
            stderr: subprocess::Redirection::Pipe,
            ..Default::default()
        },
    )?;
    p.wait_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

pub fn check_podman() -> Result<()> {
    let mut p = Popen::create(
        &["podman", "ps"],
        PopenConfig {
            stdout: subprocess::Redirection::Pipe,
            stderr: subprocess::Redirection::Pipe,
            ..Default::default()
        },
    )?;
    p.wait_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

pub fn get_docker_executor() -> Result<String> {
    if check_docker().is_ok() {
        return Ok("docker".to_string());
    } else if check_podman().is_ok() {
        return Ok("podman".to_string());
    }
    Err(anyhow::anyhow!("Docker or Podman not found."))
}

pub fn run(
    project_root: &Path,
    force_rebuild: bool,
    version: String,
    fs_map: Vec<String>,
    args: Vec<String>,
) -> Result<()> {
    let executor = get_docker_executor()?;

    let pack = Pack::load(project_root)?;
    if pack.artifacts.contains(&super::package::Artifact {
        atype: super::package::AType::Docker,
        name: "".to_string(),
        path: None,
    }) {}

    let image = build_local(project_root, version)?;

    let command = format!(
        "{} run -it {} --rm {} {}",
        executor,
        get_fs_map_str(fs_map),
        image,
        args.join(" ")
    );
    debug!("Running command: {}", command);
    let mut p = Popen::create(
        command.split_whitespace().collect::<Vec<&str>>().as_slice(),
        PopenConfig::default(),
    )?;
    p.wait()?;
    Ok(())
}

fn get_fs_map_str(fs_map: Vec<String>) -> String {
    if fs_map.is_empty() {
        return "".to_string();
    }
    let fs_map_string = String::from("-v");
    format!("{} {}", fs_map_string, fs_map.join(" -v "))
}

fn build_local(project_root: &Path, version: String) -> Result<String> {
    let executor = get_docker_executor()?;
    let pack = super::package::Pack::load(project_root)?;

    let name_str = project_root.to_str().unwrap();
    let name_str = name_str.replace("/", "-").replace(".", "-");

    let image = format!(
        "envy-{}:{}",
        pack.name.to_lowercase(),
        version.to_lowercase()
    );
    let dockerfile_path = project_root.join(".envy").join("Dockerfile");
    debug!("Building local docker image: {}", image);
    let mut popen_conf = PopenConfig {
        stdout: subprocess::Redirection::Pipe,
        stderr: subprocess::Redirection::Pipe,
        ..Default::default()
    };
    if log_enabled!(log::Level::Debug) {
        // This prints all logs
        popen_conf = PopenConfig::default();
    }
    let mut p = Popen::create(
        &[
            executor.as_str(),
            "build",
            "-t",
            image.as_str(),
            "-f",
            dockerfile_path.to_str().unwrap(),
            project_root.to_str().unwrap(),
        ],
        popen_conf,
    )?;
    let status = p.wait_timeout(std::time::Duration::from_secs(300))?;

    match status {
        Some(s) => {
            if s.success() {
                Ok(image)
            } else {
                Err(anyhow::anyhow!("Failed to build docker image."))
            }
        }
        None => Err(anyhow::anyhow!("Failed to build docker image.")),
    }
}

pub fn generate_dockerfile(pack: &Pack, project_root: &Path) -> Result<String> {
    let mut handlebars = Handlebars::new();
    let source = TEMPLATE_DOCKERFILE;
    handlebars.register_template_string("Dockerfile", source)?;

    #[derive(Default, Serialize, Deserialize)]
    struct Data {
        interpreter: String,
        entrypoint: String,
        os_deps: Vec<String>,
        ptype: PType,
        type_reqs: bool,
    }

    // trim env prefix on interpreter
    let interpreter = pack.interpreter.trim_start_matches("/usr/bin/env ");

    let mut d = Data {
        interpreter: interpreter.to_string(),
        entrypoint: pack.entrypoint.to_str().unwrap().to_string(),
        os_deps: pack.deps.clone(),
        ptype: pack.ptype.clone(),
        type_reqs: false,
    };

    // Figure out type specific deps
    match d.ptype {
        PType::Python => {
            d.type_reqs = utils::check_requirements_txt(project_root);
        }
        PType::Node => {
            d.type_reqs = utils::check_package_json(project_root);
        }
        _ => {}
    };

    Ok(handlebars.render("Dockerfile", &d)?)
}

pub fn generate_docker_ignore(pack: &Pack) -> Result<String> {
    let mut handlebars = Handlebars::new();
    let source = DOCKER_IGNORE;
    handlebars.register_template_string("dockerignore", source)?;

    #[derive(Default, Serialize, Deserialize)]
    struct Data {
        ptype: PType,
    }

    let d = Data {
        ptype: pack.ptype.clone(),
    };

    Ok(handlebars.render("dockerignore", &d)?)
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_docker_volumes_map() {
        let input = vec!["/root:/root".to_string()];
        assert_eq!(super::get_fs_map_str(input), "-v /root:/root");

        let input = vec!["/root:/root".to_string(), ".app:/app".to_string()];
        assert_eq!(super::get_fs_map_str(input), "-v /root:/root -v .app:/app");
    }
}
