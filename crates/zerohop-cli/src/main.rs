//! Command-line version of Zerohop.
//!
//! Zerohop has not been independently audited.

use std::future::Future;
use std::io::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use tokio::net::TcpListener;
use zerohop_core::address::LocalNetworks;
use zerohop_core::circle::{Circle, generate_key};
use zerohop_core::ledger::{DeviceId, Platform};
use zerohop_core::names::display_name;
use zerohop_core::node::{Node, NodeError};
use zerohop_core::pairing::{self, HostPin, PairingPrompt, PairingSecret, QrPayload, short_id};
use zerohop_core::receive::ReceiveFolder;
use zerohop_core::session::{CircleHandle, LocalInfo, SessionError, lock};
use zerohop_core::transfer::{
    self, Approver, Budgets, DEFAULT_AUTO_LIMIT, OfferOutcome, OfferSummary, ReceiveConfig,
    ReceiveMode, ReceiveSettings, SendReport, TransferError,
};
use zerohop_core::wire::PeerInfo;

const DEFAULT_PORT: u16 = 53_318;
const DEFAULT_PAIRING_PORT: u16 = 53_319;

/// Private file transfer between your own devices.
#[derive(Parser)]
#[command(name = "zerohop", version)]
struct Cli {
    /// Where this device keeps its keys and state [default: ~/.zerohop]
    #[arg(long, global = true, value_name = "DIR")]
    state: Option<PathBuf>,

    /// Treat 127.0.0.1 as a local network. For development only.
    #[cfg(feature = "dev-loopback")]
    #[arg(long, global = true)]
    allow_loopback: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start a new circle on this device
    Init {
        /// The name other devices will see
        #[arg(long)]
        name: String,
    },
    /// Add a device to the circle, or join one
    Pair {
        #[command(subcommand)]
        command: PairCommand,
    },
    /// List the devices in the circle
    Devices,
    /// Remove a device from the circle
    Revoke {
        /// Start of the device's ID, as shown by `devices`
        device: String,
    },
    /// Receive files until stopped
    Listen {
        /// Port to listen on
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        /// Where received files go [default: ~/Downloads]
        #[arg(long, value_name = "DIR")]
        folder: Option<PathBuf>,
        /// When to ask before accepting
        #[arg(long, value_enum, default_value_t = Mode::ExceptPrograms)]
        mode: Mode,
    },
    /// Send files or folders to a device
    Send {
        /// Start of the device's ID, as shown by `devices`
        device: String,
        /// The device's address, as ip:port
        address: SocketAddr,
        /// Files and folders to send
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// Continue an interrupted transfer
    Resume {
        /// Start of the device's ID, as shown by `devices`
        device: String,
        /// The device's address, as ip:port
        address: SocketAddr,
        /// Identifier printed when the transfer stopped, or shown by `unfinished`
        transfer: String,
    },
    /// List interrupted transfers that can be continued
    Unfinished,
}

#[derive(Subcommand)]
enum PairCommand {
    /// Add a new device to this circle
    Host {
        /// Port to listen on while pairing
        #[arg(long, default_value_t = DEFAULT_PAIRING_PORT)]
        port: u16,
        /// Print a pairing link instead of a six-digit code
        #[arg(long)]
        link: bool,
    },
    /// Join a circle from a new device
    Join {
        /// The host's ip:port, or a zerohop:// pairing link
        target: String,
        /// The six-digit code shown on the host. Not needed with a link.
        code: Option<String>,
        /// The name other devices will see
        #[arg(long)]
        name: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    /// Ask before every transfer
    Ask,
    /// Accept everything without asking
    Auto,
    /// Accept without asking, but ask when programs are included
    ExceptPrograms,
}

impl From<Mode> for ReceiveMode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Ask => Self::AlwaysAsk,
            Mode::Auto => Self::Automatic,
            Mode::ExceptPrograms => Self::AutomaticExceptPrograms,
        }
    }
}

struct Env {
    state: PathBuf,
    nets: LocalNetworks,
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("could not find the home folder")
}

const fn this_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::MacOs
    } else if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::Other
    }
}

fn require_circle(state: &Path) -> Result<()> {
    if Circle::exists(state) {
        Ok(())
    } else {
        bail!("this device is not in a circle yet: run `zerohop init` or `zerohop pair join`")
    }
}

/// Loads the circle as its only writer, for anything that changes it.
fn own_circle(state: &Path) -> Result<Circle> {
    require_circle(state)?;
    Ok(Circle::load(state)?)
}

fn own_name(circle: &Circle) -> String {
    circle
        .ledger()
        .name_of(&circle.device_id())
        .unwrap_or("device")
        .to_string()
}

impl Env {
    fn node(&self, circle: Circle) -> Node {
        let info = LocalInfo {
            name: own_name(&circle),
            platform: this_platform(),
        };
        Node::new(circle, self.nets.clone(), info, &self.state)
    }

    /// A node for sending, which also works while `listen` owns the state.
    fn sending_node(&self) -> Result<Node> {
        require_circle(&self.state)?;
        Ok(self.node(Circle::open(&self.state)?))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_transfer_id(text: &str) -> Result<[u8; 16]> {
    let digits = text.as_bytes();
    if digits.len() != 32 {
        bail!("a transfer ID has 32 hexadecimal digits");
    }
    let mut id = [0u8; 16];
    for (byte, pair) in id.iter_mut().zip(digits.as_chunks::<2>().0) {
        let pair = std::str::from_utf8(pair).context("invalid transfer ID")?;
        *byte = u8::from_str_radix(pair, 16).context("invalid transfer ID")?;
    }
    Ok(id)
}

fn resolve_device(circle: &CircleHandle, prefix: &str) -> Result<DeviceId> {
    let wanted = prefix.to_lowercase();
    let circle = lock(circle)?;
    let view = circle.ledger().membership();
    let mut matches = view
        .ever
        .iter()
        .filter(|id| hex(id.as_slice()).starts_with(&wanted));
    let first = matches
        .next()
        .ok_or_else(|| anyhow!("no device starts with {prefix}"))?;
    if matches.next().is_some() {
        bail!("more than one device starts with {prefix}: use more digits");
    }
    Ok(*first)
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    #[expect(clippy::cast_precision_loss, reason = "only shown to a person")]
    let mut value = bytes as f64;
    let mut unit = UNITS[0];
    for next in &UNITS[1..] {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next;
    }
    if unit == UNITS[0] {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {unit}")
    }
}

async fn ask(question: String) -> bool {
    tokio::task::spawn_blocking(move || {
        print!("{question} [y/N] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).is_ok()
            && matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
    })
    .await
    .unwrap_or(false)
}

async fn confirm_pairing(prompt: PairingPrompt) -> bool {
    // The name comes from a device that has not been approved yet.
    println!();
    println!(
        "Other device: {} ({:?}, {})",
        display_name(&prompt.peer_name),
        prompt.peer_platform,
        prompt.peer_short_id
    );
    println!("Comparison number: {}", prompt.comparison);
    ask("Does the other device show the same number?".to_string()).await
}

fn init(state: &Path, name: &str) -> Result<()> {
    if Circle::exists(state) {
        bail!("this device already has a circle in {}", state.display());
    }
    std::fs::create_dir_all(state)?;
    let circle = Circle::create(state, name)?;
    println!(
        "Created a circle. This device is {}.",
        short_id(&circle.device_id())
    );
    Ok(())
}

fn print_how_to_reach(nets: &LocalNetworks, port: u16) {
    for ip in nets.addresses().into_iter().filter(IpAddr::is_ipv4) {
        println!("Address: {ip}:{port}");
    }
}

async fn pair_host(ctx: &Env, port: u16, link: bool) -> Result<()> {
    let mut circle = own_circle(&ctx.state)?;
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await?;
    let port = listener.local_addr()?.port();
    let secret = if link {
        PairingSecret::new_qr()?
    } else {
        PairingSecret::new_code()?
    };
    if let Some(bytes) = secret.qr_bytes() {
        let addresses = ctx
            .nets
            .addresses()
            .into_iter()
            .filter(IpAddr::is_ipv4)
            .map(|ip| SocketAddr::new(ip, port))
            .collect();
        let payload = QrPayload {
            host: circle.device_id(),
            secret: bytes,
            addresses,
        };
        println!("Pairing link: {}", payload.encode());
    } else {
        println!(
            "Pairing code: {}",
            secret.display_code().unwrap_or_default()
        );
    }
    print_how_to_reach(&ctx.nets, port);
    println!("Waiting for the other device...");
    let info = PeerInfo {
        name: own_name(&circle),
        platform: this_platform(),
    };
    let paired = pairing::host(
        listener,
        &ctx.nets,
        &mut circle,
        &secret,
        info,
        confirm_pairing,
    )
    .await?;
    println!(
        "Added {} ({}).",
        display_name(&paired.name),
        short_id(&paired.device)
    );
    Ok(())
}

async fn pair_join(ctx: &Env, target: &str, code: Option<&str>, name: &str) -> Result<()> {
    if Circle::exists(&ctx.state) {
        bail!(
            "this device already has a circle in {}",
            ctx.state.display()
        );
    }
    std::fs::create_dir_all(&ctx.state)?;
    let (addresses, secret, pin) = if target.starts_with("zerohop://") {
        let link = QrPayload::decode(target).context("this pairing link is not valid")?;
        (
            link.addresses,
            PairingSecret::from_qr_bytes(link.secret),
            HostPin::Key(link.host),
        )
    } else {
        let code = code.context("enter the six-digit code shown on the other device")?;
        (
            vec![
                target
                    .parse()
                    .context("the address must look like 192.168.1.20:53319")?,
            ],
            PairingSecret::from_typed(code),
            HostPin::None,
        )
    };
    let address = *addresses
        .first()
        .context("this pairing link has no address")?;
    let info = PeerInfo {
        name: name.to_string(),
        platform: this_platform(),
    };
    let key = generate_key()?;
    let circle = pairing::join(
        address,
        &ctx.nets,
        &ctx.state,
        key,
        &secret,
        pin,
        info,
        confirm_pairing,
    )
    .await?;
    println!("Joined. This device is {}.", short_id(&circle.device_id()));
    Ok(())
}

fn devices(state: &Path) -> Result<()> {
    require_circle(state)?;
    let circle = Circle::snapshot(state)?;
    let view = circle.ledger().membership();
    let me = circle.device_id();
    let name_of = |id: &DeviceId| display_name(circle.ledger().name_of(id).unwrap_or("unknown"));
    for id in &view.members {
        let marker = if *id == me { "  (this device)" } else { "" };
        println!("{}  {}{marker}", short_id(id), name_of(id));
    }
    for id in &view.revoked {
        println!("{}  {}  (removed)", short_id(id), name_of(id));
    }
    Ok(())
}

fn revoke(state: &Path, device: &str) -> Result<()> {
    let handle: CircleHandle = Arc::new(std::sync::Mutex::new(own_circle(state)?));
    let id = resolve_device(&handle, device)?;
    let mut circle = lock(&handle)?;
    let name = display_name(circle.ledger().name_of(&id).unwrap_or("unknown"));
    circle.revoke(id)?;
    println!(
        "Removed {name}. Other devices learn about it the next time they connect to this one."
    );
    Ok(())
}

struct Terminal {
    circle: CircleHandle,
}

impl Terminal {
    fn name_of(&self, peer: &DeviceId) -> String {
        let name = lock(&self.circle)
            .ok()
            .and_then(|c| c.ledger().name_of(peer).map(str::to_string));
        display_name(name.as_deref().unwrap_or("unknown"))
    }
}

impl Approver for Terminal {
    fn approve(&self, offer: OfferSummary) -> Pin<Box<dyn Future<Output = bool> + Send>> {
        let program = if offer.has_program {
            ", including programs"
        } else {
            ""
        };
        let question = format!(
            "{} wants to send {} file(s), {}{program}. Accept?",
            self.name_of(&offer.peer),
            offer.files,
            format_size(offer.total_bytes),
        );
        Box::pin(ask(question))
    }

    fn finished(&self, peer: &DeviceId, _transfer: &[u8; 16], outcome: &OfferOutcome) {
        let name = self.name_of(peer);
        match outcome {
            OfferOutcome::Received(counts) => {
                println!("Received from {name}: {} item(s) saved.", counts.published);
                if counts.held > 0 {
                    println!(
                        "{} item(s) contain programs disguised as other files and were kept out of the folder.",
                        counts.held
                    );
                }
            }
            OfferOutcome::Declined(reason) => println!("Declined an offer from {name} ({reason})."),
        }
    }
}

async fn listen(ctx: &Env, port: u16, folder: Option<PathBuf>, mode: Mode) -> Result<()> {
    let node = ctx.node(own_circle(&ctx.state)?);
    let folder = match folder {
        Some(path) => path,
        None => home()?.join("Downloads"),
    };
    std::fs::create_dir_all(&folder)?;
    let cfg = ReceiveConfig {
        settings: ReceiveSettings::new(ReceiveFolder::open(&folder)?, mode.into()),
        approver: Arc::new(Terminal {
            circle: node.circle().clone(),
        }),
        auto_limit: DEFAULT_AUTO_LIMIT,
        budgets: Arc::new(Budgets::default()),
        clip: None,
    };
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await?;
    print_how_to_reach(&ctx.nets, listener.local_addr()?.port());
    println!("Saving to {}. Press Ctrl-C to stop.", folder.display());
    tokio::select! {
        result = node.serve(listener, cfg) => result?,
        _ = tokio::signal::ctrl_c() => {
            // A question may be waiting on the terminal, which cannot be interrupted.
            std::process::exit(0);
        }
    }
    Ok(())
}

fn print_report(report: SendReport, started: Instant) {
    let seconds = started.elapsed().as_secs_f64().max(0.001);
    #[expect(clippy::cast_precision_loss, reason = "only shown to a person")]
    let speed = report.bytes_sent as f64 / seconds / (1024.0 * 1024.0);
    println!(
        "Sent {} in {seconds:.1} s ({speed:.0} MiB/s).",
        format_size(report.bytes_sent)
    );
    if report.held > 0 {
        println!(
            "The other device kept {} item(s) back for review.",
            report.held
        );
    }
}

const fn can_continue(error: &NodeError) -> bool {
    !matches!(
        error,
        NodeError::Tls(_)
            | NodeError::Session(SessionError::Removed | SessionError::NotMember)
            | NodeError::Transfer(TransferError::Declined(_) | TransferError::SourceChanged)
    )
}

async fn send(ctx: &Env, device: &str, address: SocketAddr, paths: &[PathBuf]) -> Result<()> {
    let node = ctx.sending_node()?;
    let peer = resolve_device(node.circle(), device)?;
    let outgoing = transfer::plan(paths)?;
    let id = hex(&outgoing.transfer_id);
    let started = Instant::now();
    match node.send(peer, address, outgoing).await {
        Ok(report) => {
            print_report(report, started);
            Ok(())
        }
        Err(error) => {
            if can_continue(&error) {
                eprintln!("To continue: zerohop resume {device} {address} {id}");
            }
            Err(error.into())
        }
    }
}

async fn resume(ctx: &Env, device: &str, address: SocketAddr, transfer: &str) -> Result<()> {
    let node = ctx.sending_node()?;
    let peer = resolve_device(node.circle(), device)?;
    let id = parse_transfer_id(transfer)?;
    let started = Instant::now();
    print_report(node.resume(peer, address, id).await?, started);
    Ok(())
}

fn list_unfinished(state: &Path) -> Result<()> {
    let list = transfer::unfinished(state)?;
    if list.is_empty() {
        println!("Nothing to continue.");
    }
    for entry in list {
        println!(
            "{}  to {}  {}",
            hex(&entry.transfer_id),
            short_id(&entry.peer),
            format_size(entry.total_bytes)
        );
    }
    Ok(())
}

async fn run(cli: Cli) -> Result<()> {
    let state = match cli.state {
        Some(path) => path,
        None => home()?.join(".zerohop"),
    };
    let nets = LocalNetworks::detect()?;
    #[cfg(feature = "dev-loopback")]
    let nets = if cli.allow_loopback {
        nets.with_loopback()
    } else {
        nets
    };
    let ctx = Env { state, nets };

    match cli.command {
        Command::Init { name } => init(&ctx.state, &name),
        Command::Pair {
            command: PairCommand::Host { port, link },
        } => pair_host(&ctx, port, link).await,
        Command::Pair {
            command: PairCommand::Join { target, code, name },
        } => pair_join(&ctx, &target, code.as_deref(), &name).await,
        Command::Devices => devices(&ctx.state),
        Command::Revoke { device } => revoke(&ctx.state, &device),
        Command::Listen { port, folder, mode } => listen(&ctx, port, folder, mode).await,
        Command::Send {
            device,
            address,
            paths,
        } => send(&ctx, &device, address, &paths).await,
        Command::Resume {
            device,
            address,
            transfer,
        } => resume(&ctx, &device, address, &transfer).await,
        Command::Unfinished => list_unfinished(&ctx.state),
    }
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
