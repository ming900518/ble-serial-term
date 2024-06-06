use bleasy::{ScanConfig, Scanner};
use clap::{Parser, Subcommand};
use futures::StreamExt;
use std::sync::{Arc, OnceLock};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    spawn,
    sync::{broadcast, mpsc, Mutex},
};
use uuid::{uuid, Uuid};

const RX_CHARACTERISTIC: Uuid = uuid!("6E400002-B5A3-F393-E0A9-E50E24DCCA9E");
const TX_CHARACTERISTIC: Uuid = uuid!("6E400003-B5A3-F393-E0A9-E50E24DCCA9E");

static BT_SEND_DATA_QUEUE: OnceLock<mpsc::Sender<Vec<u8>>> = OnceLock::new();
static BT_RECEIVED_DATA_CHANNEL: OnceLock<broadcast::Sender<Vec<u8>>> = OnceLock::new();

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    subcommand: Subcommands,
}

#[derive(Subcommand, Clone)]
enum Subcommands {
    Scan,
    Connect {
        #[arg(short, long)]
        ble_name: String,
        #[arg(short, long)]
        port: u16,
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
        Subcommands::Connect { ble_name, port } => {
            let mut device = None;

            while let Some(found_device) = scanner.device_stream().next().await {
                let found_device_name = found_device.local_name().await.unwrap_or_default();
                if found_device_name == ble_name {
                    device = Some(found_device);
                    println!("BLE connected: {found_device_name}");
                    break;
                }
            }

            let device = device.unwrap();

            let (bt_tx, bt_rx) = (
                Arc::new(Mutex::new(
                    device.characteristic(TX_CHARACTERISTIC).await?.unwrap(),
                )),
                Arc::new(Mutex::new(
                    device.characteristic(RX_CHARACTERISTIC).await?.unwrap(),
                )),
            );

            let (bt_tx, bt_rx) = (bt_tx.clone(), bt_rx.clone());

            let (bt_recv_tx, _bt_recv_rx) = tokio::sync::broadcast::channel(10);
            BT_RECEIVED_DATA_CHANNEL.get_or_init(|| bt_recv_tx.clone());

            let (bt_sent_tx, mut bt_sent_rx) = tokio::sync::mpsc::channel(10);
            let bt_sent_tx = BT_SEND_DATA_QUEUE.get_or_init(|| bt_sent_tx);

            spawn(async move {
                while let Some(data) = bt_sent_rx.recv().await {
                    bt_rx.lock().await.write_command(&data).await.unwrap();
                }
            });

            spawn(async move {
                while let Ok(data) = bt_tx.lock().await.read().await {
                    if !data.is_empty() {
                        bt_recv_tx.send(data.clone()).unwrap();
                        bt_sent_tx.send(vec![b'\0']).await.unwrap();
                    }
                }
            });

            let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await?;

            while let Ok((socket, _)) = listener.accept().await {
                let bt_send_data_queue = BT_SEND_DATA_QUEUE.get().unwrap();
                let mut bt_receiver = BT_RECEIVED_DATA_CHANNEL.get().unwrap().subscribe();
                let (mut tcp_tx, mut tcp_rx) = socket.into_split();

                spawn(async move {
                    let mut buf = [0; 1024];
                    while let Ok(_) = tcp_tx.readable().await {
                        let len = tcp_tx.read(&mut buf).await.unwrap();
                        bt_send_data_queue
                            .send(buf.split_at(len).0.to_vec())
                            .await
                            .ok();
                        buf = [0; 1024];
                    }
                });

                spawn(async move {
                    while let Ok(data) = bt_receiver.recv().await {
                        tcp_rx.write(&data).await.ok();
                    }
                });
            }
        }
    }
    Ok(())
}
