//! Instance of nym client
use rand::Rng;
use std::{net::TcpListener, ops::Range, process::Stdio};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

const LOCALHOST: &str = "0.0.0.0";
const PORT_RANGE: Range<u16> = 15000..25000;

/// Pick a random port
fn pick_port() -> u16 {
    let mut rng = rand::thread_rng();

    loop {
        let port = rng.gen_range(PORT_RANGE);
        if TcpListener::bind(format!("{LOCALHOST}:{port}")).is_ok() {
            return port;
        }
    }
}

/// Instance of nym client
pub struct NymClient {
    ps: Child,
    pub port: u16,
}

impl NymClient {
    pub async fn start() -> Self {
        let port = pick_port();
        let id = port.to_string();

        // init nym client.
        Command::new("nym-client")
            .arg("init")
            .arg("--id")
            .arg(&id)
            .output()
            .await
            .expect("Failed to init nym-client");

        // start nym-client
        let mut ps = Command::new("nym-client")
            .kill_on_drop(true)
            .arg("run")
            .arg("--id")
            .arg(&id)
            .arg("--port")
            .arg(&id)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to start nym-client");

        // wait for connection established
        let stderr = ps.stderr.take().expect("Failed to get stderr");
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            println!("{line}");
            if line.contains("The address of this client is") {
                break;
            }
        }

        Self { ps, port }
    }

    pub fn process(self) -> Child {
        self.ps
    }
}
