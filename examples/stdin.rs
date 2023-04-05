use futures::prelude::*;
use std::{io::BufReader, time::Duration};
use tokio::{sync::oneshot, *}; // 0.1.27 // 0.1.21

fn main() {
    let (tx, rx) = oneshot::channel::<()>();
    let lines = tokio::io::lines(BufReader::new(tokio::io::stdin()));
    let lines = lines.for_each(|item| {
        println!("> {:?}", item);
        Ok(())
    });

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(5000));
        println!("system shutting down");
        let _ = tx.send(());
    });

    let lines = lines.select2(rx);

    tokio::run(lines.map(|_| ()).map_err(|_| ()));
}
