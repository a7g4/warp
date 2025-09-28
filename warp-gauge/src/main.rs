const PACKET_SIZE: usize = 1000;

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufWriter, Write};

mod inspector;

#[derive(clap::Parser)]
#[command(name = "warp-gauge")]
#[command(about = "Benchmark latency v/s bandwidth for a link")]
struct Args {
    #[command(subcommand)]
    mode: Option<Mode>,
}

#[derive(Debug, Clone, clap::Subcommand)]
enum Mode {
    // This configures the transmitter to generate load as a sawtooth:
    // - Base packets per second, ramping up to peak packets per second over "period" seconds before resetting back to base packets per second
    Tx {
        destination: String,
        peak_pps: u64,
        base_pps: u64,
        period: u64,
    },
    Rx {
        destination: String,
        output_path: String,
    },
    // Default
    Inspector,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DataPoint {
    counter: u64,
    target_pps: u64,
    sender_achieved_pps: u64,
    receiver_calculated_pps: u64,
    latency_ms: f64,
}

#[derive(Clone)]
enum DestinationAddress {
    Ip(std::net::SocketAddr),
    Uds(std::path::PathBuf),
}

enum SenderSocket {
    Ip(tokio::net::UdpSocket),
    Uds(tokio::net::UnixDatagram),
}

impl SenderSocket {
    fn new(address: DestinationAddress) -> Result<Self, anyhow::Error> {
        match address {
            DestinationAddress::Ip(_) => {
                let std_socket = std::net::UdpSocket::bind("0.0.0.0:0")?;
                std_socket.set_nonblocking(true)?;
                Ok(Self::Ip(tokio::net::UdpSocket::from_std(std_socket)?))
            }
            DestinationAddress::Uds(path) => {
                let temp_path = format!("/tmp/warp-bench-tx-{}", std::process::id());
                std::fs::remove_file(&temp_path).ok();
                let socket = tokio::net::UnixDatagram::bind(&temp_path)?;
                socket.connect(&path)?;
                Ok(Self::Uds(socket))
            }
        }
    }
}
enum ReceiverSocket {
    Ip(tokio::net::UdpSocket),
    Uds(tokio::net::UnixDatagram),
}

impl ReceiverSocket {
    fn new(address: DestinationAddress) -> Result<Self, anyhow::Error> {
        match address {
            DestinationAddress::Ip(socket_addr) => {
                let std_socket = std::net::UdpSocket::bind(socket_addr)?;
                std_socket.set_nonblocking(true)?;
                Ok(Self::Ip(tokio::net::UdpSocket::from_std(std_socket)?))
            }
            DestinationAddress::Uds(path) => {
                // For receiver, bind to the specified path
                std::fs::remove_file(&path).ok(); // Ignore if file doesn't exist
                Ok(Self::Uds(tokio::net::UnixDatagram::bind(&path)?))
            }
        }
    }
}

struct Receiver {
    socket: ReceiverSocket,
    rx_timestamps: std::collections::VecDeque<std::time::SystemTime>,
}

impl Receiver {
    fn new(address: DestinationAddress) -> Result<Self, anyhow::Error> {
        Ok(Receiver {
            socket: ReceiverSocket::new(address)?,
            rx_timestamps: Default::default(),
        })
    }

    async fn receive(&mut self, file: &mut std::io::BufWriter<File>, buf: &mut [u8]) -> Result<(), anyhow::Error> {
        let len = match &self.socket {
            ReceiverSocket::Ip(socket) => socket.recv_from(buf).await?.0,
            ReceiverSocket::Uds(socket) => socket.recv(buf).await?,
        };
        if len == PACKET_SIZE {
            let receive_time = std::time::SystemTime::now();
            let payload: Payload = bincode::decode_from_slice(buf, bincode::config::standard())?.0;

            while let Some(&front_time) = self.rx_timestamps.front() {
                if receive_time
                    .duration_since(front_time)
                    .unwrap_or(std::time::Duration::from_secs(0))
                    >= std::time::Duration::from_secs(1)
                {
                    self.rx_timestamps.pop_front();
                } else {
                    break;
                }
            }

            self.rx_timestamps.push_back(receive_time);
            let receiver_pps = self.rx_timestamps.len() as u64;
            let latency = receive_time
                .duration_since(payload.timestamp)
                .map(|d| d.as_secs_f64())
                .unwrap_or_else(|d| -d.duration().as_secs_f64());

            writeln!(
                file,
                "{},{},{},{},{}",
                payload.counter,
                payload.target_packets_per_second,
                payload.achieved_packets_per_second,
                receiver_pps,
                latency
            )?;
        }
        Ok(())
    }
}

struct Sender {
    socket: SenderSocket,
    destination: DestinationAddress,
    tx_timestamps: std::collections::VecDeque<std::time::SystemTime>,
    counter: u64,
    target_packets_per_second: u64,
    base_pps: u64,
    peak_pps: u64,
    period: u64,
    start_time: std::time::SystemTime,
    last_period_report: u64,
}

#[derive(bincode::Encode, bincode::Decode, Clone)]
struct Payload {
    counter: u64,
    timestamp: std::time::SystemTime,
    target_packets_per_second: u64,
    achieved_packets_per_second: u64,
}

impl Sender {
    fn new(destination: DestinationAddress, base_pps: u64, peak_pps: u64, period: u64) -> Result<Self, anyhow::Error> {
        Ok(Sender {
            socket: SenderSocket::new(destination.clone())?,
            destination,
            tx_timestamps: Default::default(),
            counter: 0,
            target_packets_per_second: base_pps,
            base_pps,
            peak_pps,
            period,
            start_time: std::time::SystemTime::now(),
            last_period_report: 0,
        })
    }

    fn update_target(&mut self) {
        let elapsed_total = self.start_time.elapsed().unwrap().as_secs();
        let elapsed = elapsed_total % self.period;
        let fraction = elapsed as f64 / self.period as f64;
        self.target_packets_per_second = self.base_pps + ((self.peak_pps - self.base_pps) as f64 * fraction) as u64;

        let current_period = elapsed_total / self.period;
        if current_period > self.last_period_report {
            println!("Period {current_period}");
            self.last_period_report = current_period;
        }
    }

    async fn send(&mut self) -> Result<(), anyhow::Error> {
        let current_time = std::time::SystemTime::now();
        while let Some(t) = self.tx_timestamps.front() {
            if current_time.duration_since(*t)? >= std::time::Duration::from_secs(1) {
                self.tx_timestamps.pop_front();
            } else {
                break;
            }
        }

        self.counter += 1;
        let payload = Payload {
            counter: self.counter,
            timestamp: current_time,
            target_packets_per_second: self.target_packets_per_second,
            achieved_packets_per_second: self.tx_timestamps.len() as u64,
        };

        let mut payload = bincode::encode_to_vec(payload, bincode::config::standard())?;
        payload.resize(PACKET_SIZE, b'*');
        let sent_bytes = match &self.socket {
            SenderSocket::Ip(socket) => {
                if let DestinationAddress::Ip(addr) = &self.destination {
                    socket.send_to(payload.as_slice(), *addr).await
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Invalid destination",
                    ))
                }
            }
            SenderSocket::Uds(socket) => socket.send(payload.as_slice()).await,
        };
        match sent_bytes {
            Ok(len) if len == PACKET_SIZE => {
                self.tx_timestamps.push_back(current_time);
                Ok(())
            }
            Ok(len) => Err(anyhow::anyhow!("Only sent {} bytes of {}", len, PACKET_SIZE)),
            Err(e) => Err(anyhow::Error::new(e)),
        }
    }
}

fn parse_destination(s: &str) -> Result<DestinationAddress, anyhow::Error> {
    if let Ok(addr) = s.parse::<std::net::SocketAddr>() {
        Ok(DestinationAddress::Ip(addr))
    } else {
        Ok(DestinationAddress::Uds(std::path::PathBuf::from(s)))
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();
    match args.mode {
        Some(Mode::Tx {
            destination,
            peak_pps,
            base_pps,
            period,
        }) => {
            let dest = parse_destination(&destination)?;
            let mut sender = Sender::new(dest, base_pps, peak_pps, period)?;
            run_tx(&mut sender).await?;
        }
        Some(Mode::Rx {
            destination,
            output_path,
        }) => {
            let dest = parse_destination(&destination)?;
            let mut receiver = Receiver::new(dest)?;
            run_rx(&mut receiver, &output_path).await?;
        }
        Some(Mode::Inspector) | None => {
            let options = eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 600.0]),
                ..Default::default()
            };

            eframe::run_native(
                "Warp Guage",
                options,
                Box::new(|_cc| {
                    let inspector = inspector::Inspector::default();
                    Ok(Box::<crate::inspector::Inspector>::new(inspector))
                }),
            )
            .expect("GUI error");
        }
    }
    Ok(())
}

async fn run_tx(sender: &mut Sender) -> Result<(), anyhow::Error> {
    println!(
        "Starting sender: base_pps={}, peak_pps={}, period={}",
        sender.base_pps, sender.peak_pps, sender.period
    );
    use std::io::Write;
    std::io::stdout().flush().unwrap();

    let mut next_send_time = tokio::time::Instant::now();
    let mut last_debug_time = 0u64;

    loop {
        sender.update_target();

        let elapsed = sender.start_time.elapsed().unwrap().as_secs();
        if elapsed > last_debug_time {
            println!(
                "Debug: {}s - Target PPS: {}, Achieved PPS: {}, Counter: {}",
                elapsed,
                sender.target_packets_per_second,
                sender.tx_timestamps.len(),
                sender.counter
            );
            last_debug_time = elapsed;
        }

        let interval = tokio::time::Duration::from_secs_f64(1.0 / sender.target_packets_per_second as f64);

        // Wait until it's time to send
        let now = tokio::time::Instant::now();
        if now < next_send_time {
            let sleep_time = next_send_time - now;
            if sleep_time >= tokio::time::Duration::from_millis(1) {
                tokio::time::sleep_until(next_send_time).await;
            } else {
                // Busy wait for sub-millisecond precision
                while tokio::time::Instant::now() < next_send_time {
                    tokio::task::yield_now().await;
                }
            }
        }

        sender.send().await?;
        next_send_time += interval;

        // Prevent drift - if we're behind, catch up
        let now = tokio::time::Instant::now();
        if next_send_time < now {
            next_send_time = now;
        }
    }
}

async fn run_rx(receiver: &mut Receiver, output_path: &str) -> Result<(), anyhow::Error> {
    let file = File::create(output_path)?;
    let mut buf_writer = BufWriter::with_capacity(64 * 1024, file);
    writeln!(
        buf_writer,
        "counter,target_pps,sender_achieved_pps,receiver_calculated_pps,latency_ms"
    )?;

    let mut buf = vec![0u8; PACKET_SIZE];

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                buf_writer.flush()?;
                break;
            },
            _ = receiver.receive(&mut buf_writer, &mut buf) => {},
        }
    }
    Ok(())
}
