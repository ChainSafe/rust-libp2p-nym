// This section instantiates docker containers of the nym-client
// so that tests can be run with all the necessary resources.
// This removes the requirement for having to limit test threads
// or to build/run nym-client ourselves.
#[macro_export]
macro_rules! new_nym_client {
    ($nym_id:ident, $uri:ident) => {
        let docker_client = clients::Cli::default();
        let nym_ready_message = WaitFor::message_on_stderr("Client startup finished!");
        let nym_image = GenericImage::new("nym", "latest")
            .with_env_var("NYM_ID", $nym_id)
            .with_wait_for(nym_ready_message)
            .with_exposed_port(1977);
        let nym_container = docker_client.run(nym_image);
        let nym_port = nym_container.get_host_port_ipv4(1977);
        let $uri = format!("ws://0.0.0.0:{nym_port}");
    };
}
