use std::env;
use std::process::Command;
fn main() {
    println!("cargo:rerun-if-changed=Dockerfile.nym");
    let docker_build = env::var("DOCKER_BUILD").is_ok();
    if docker_build {
        let output = Command::new("docker")
            .arg("build")
            .arg("-t")
            .arg("nym:latest")
            .arg("-f")
            .arg("./Dockerfile.nym")
            .arg(".")
            .output()
            .expect("ERROR: unable to prepare context");
        println!("status: {}", output.status);
        println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success());
    };
}
