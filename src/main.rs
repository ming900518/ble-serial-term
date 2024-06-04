use std::{
    path::PathBuf,
    sync::Arc,
};

use bleasy::{ScanConfig, Scanner};
use clap::{Parser, Subcommand};
use futures::StreamExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, Interest},
    net::UnixListener,
    spawn, sync::Mutex,
};
use uuid::uuid;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    subcommand: Subcommands,
}

#[derive(Subcommand, Clone)]
enum Subcommands {
    Scan,
    Connect {
        #[arg(long = "if")]
        from_ble_name: String,
        #[arg(long = "of")]
        to_serial_path: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let config = ScanConfig::default();

    let mut scanner = Scanner::new();
    scanner.start(config).await?;

    match cli.subcommand {
        Subcommands::Scan => {
            while let Some(a) = scanner.device_stream().next().await {
                println!(
                    "{:#?}",
                    a.local_name()
                        .await
                        .unwrap_or_else(|| String::from("NO NAME"))
                )
            }
        }
        Subcommands::Connect {
            from_ble_name,
            to_serial_path,
        } => {
            let unix_listener = UnixListener::bind(to_serial_path).unwrap();
            let device = {
                let mut return_device = None;
                'inner: while let Some(device) = scanner.device_stream().next().await {
                    if device.local_name().await == Some(from_ble_name.clone()) {
                        return_device = Some(device);
                        break 'inner;
                    }
                }
                Arc::new(Mutex::new(return_device.unwrap()))
            };

            let device_1 = device.clone();

            while let Ok((stream, _)) = unix_listener.accept().await {
                let device_2 = device_1.clone();
                let device_3 = device_1.clone();
                let (mut tx, mut rx) = stream.into_split();

                spawn(async move {
                    let ble_tx = device_2.lock().await.characteristic(uuid!("6E400003-B5A3-F393-E0A9-E50E24DCCA9E")).await.unwrap().unwrap();
                    while let Ok(data) = ble_tx.read().await {
                        rx.write(&data).await.unwrap();
                    }
                });

                spawn(async move {
                    let mut buf = Vec::new();
                    loop {
                        tx.ready(Interest::READABLE).await.unwrap();
                        tx.read_to_end(&mut buf).await.unwrap();
                        device_3
                            .lock()
                            .await
                            .characteristic(uuid!("6E400002-B5A3-F393-E0A9-E50E24DCCA9E"))
                            .await
                            .unwrap()
                            .unwrap()
                            .write_request(&buf)
                            .await
                            .unwrap();
                    }
                });
            }
        }
    }
    Ok(())
}
