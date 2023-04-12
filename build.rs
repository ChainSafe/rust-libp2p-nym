use std::env;
use std::process::Command;

fn main() {
    // NOTE: This builds the Docker container *locally*,
    // and is not pulling the image from anywhere publically.
    let docker_image = "chainsafe/nym:1.1.12";

    if env::var("DOCKER_BUILD").is_ok() {
        let output = Command::new("docker")
            .arg("build")
            .arg("-t")
            .arg(docker_image)
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
