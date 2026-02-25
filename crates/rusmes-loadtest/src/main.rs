//! RusMES Load Testing CLI

use anyhow::Result;
use clap::{Parser, ValueEnum};
use rusmes_loadtest::config::{MessageContent, MessageSize, Protocol};
use rusmes_loadtest::reporter::Reporter;
use rusmes_loadtest::scenarios::ScenarioType;
use rusmes_loadtest::{LoadTestConfig, LoadTester};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rusmes-loadtest")]
#[command(about = "RusMES Load Testing Tool", long_about = None)]
#[command(version)]
struct Cli {
    /// Target host
    #[arg(short = 'H', long, default_value = "localhost")]
    host: String,

    /// Target port
    #[arg(short, long, default_value = "25")]
    port: u16,

    /// Protocol to test
    #[arg(long, value_enum, default_value = "smtp")]
    protocol: ProtocolArg,

    /// Test scenario
    #[arg(short, long, value_enum, default_value = "smtp-throughput")]
    scenario: Scenario,

    /// Test duration in seconds
    #[arg(short, long, default_value = "60")]
    duration: u64,

    /// Number of concurrent workers
    #[arg(short, long, default_value = "10")]
    concurrency: usize,

    /// Target message rate (messages per second)
    #[arg(short, long, default_value = "100")]
    rate: u64,

    /// Ramp-up duration in seconds
    #[arg(long, default_value = "0")]
    ramp_up: u64,

    /// Minimum message size in bytes
    #[arg(long, default_value = "1024")]
    min_size: usize,

    /// Maximum message size in bytes
    #[arg(long, default_value = "102400")]
    max_size: usize,

    /// Message content type
    #[arg(long, value_enum, default_value = "random")]
    content: ContentType,

    /// Output JSON report path
    #[arg(long)]
    output_json: Option<PathBuf>,

    /// Output HTML report path
    #[arg(long)]
    output_html: Option<PathBuf>,

    /// Output CSV report path
    #[arg(long)]
    output_csv: Option<PathBuf>,

    /// Enable Prometheus metrics export
    #[arg(long)]
    prometheus: bool,

    /// Prometheus export port
    #[arg(long, default_value = "9090")]
    prometheus_port: u16,

    /// SMTP weight for mixed protocol (0-100)
    #[arg(long, default_value = "70")]
    smtp_weight: u8,

    /// IMAP weight for mixed protocol (0-100)
    #[arg(long, default_value = "20")]
    imap_weight: u8,

    /// JMAP weight for mixed protocol (0-100)
    #[arg(long, default_value = "10")]
    jmap_weight: u8,

    /// POP3 weight for mixed protocol (0-100)
    #[arg(long, default_value = "0")]
    pop3_weight: u8,
}

#[derive(ValueEnum, Clone)]
enum ProtocolArg {
    Smtp,
    Imap,
    Jmap,
    Pop3,
    Mixed,
}

impl From<ProtocolArg> for Protocol {
    fn from(p: ProtocolArg) -> Self {
        match p {
            ProtocolArg::Smtp => Protocol::Smtp,
            ProtocolArg::Imap => Protocol::Imap,
            ProtocolArg::Jmap => Protocol::Jmap,
            ProtocolArg::Pop3 => Protocol::Pop3,
            ProtocolArg::Mixed => Protocol::Mixed,
        }
    }
}

#[derive(ValueEnum, Clone)]
enum Scenario {
    SmtpThroughput,
    ConcurrentConnections,
    MixedProtocol,
    SustainedLoad,
}

impl From<Scenario> for ScenarioType {
    fn from(s: Scenario) -> Self {
        match s {
            Scenario::SmtpThroughput => ScenarioType::SmtpThroughput,
            Scenario::ConcurrentConnections => ScenarioType::ConcurrentConnections,
            Scenario::MixedProtocol => ScenarioType::MixedProtocol,
            Scenario::SustainedLoad => ScenarioType::SustainedLoad,
        }
    }
}

#[derive(ValueEnum, Clone)]
enum ContentType {
    Random,
    Template,
    RealWorld,
}

impl From<ContentType> for MessageContent {
    fn from(c: ContentType) -> Self {
        match c {
            ContentType::Random => MessageContent::Random,
            ContentType::Template => MessageContent::Template,
            ContentType::RealWorld => MessageContent::RealWorld,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let protocol = cli.protocol.into();
    let mixed_weights = if protocol == Protocol::Mixed {
        Some((
            cli.smtp_weight,
            cli.imap_weight,
            cli.jmap_weight,
            cli.pop3_weight,
        ))
    } else {
        None
    };

    let config = LoadTestConfig {
        target_host: cli.host,
        target_port: cli.port,
        protocol,
        scenario: cli.scenario.into(),
        duration_secs: cli.duration,
        concurrency: cli.concurrency,
        message_rate: cli.rate,
        ramp_up_secs: cli.ramp_up,
        message_size: MessageSize::Random {
            min: cli.min_size,
            max: cli.max_size,
        },
        message_content: cli.content.into(),
        message_size_min: None,
        message_size_max: None,
        output_json: cli
            .output_json
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        output_html: cli
            .output_html
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        output_csv: cli
            .output_csv
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        prometheus_export: cli.prometheus,
        prometheus_port: cli.prometheus_port,
        mixed_weights,
    };

    config
        .validate()
        .map_err(|e| anyhow::anyhow!("Invalid configuration: {}", e))?;

    println!("Starting load test...");
    println!("Target: {}:{}", config.target_host, config.target_port);
    println!("Protocol: {:?}", config.protocol);
    println!("Scenario: {:?}", config.scenario);
    println!("Duration: {}s", config.duration_secs);
    println!("Concurrency: {}", config.concurrency);
    println!("Rate: {} msg/s", config.message_rate);
    if config.ramp_up_secs > 0 {
        println!("Ramp-up: {}s", config.ramp_up_secs);
    }
    println!();

    let tester = LoadTester::new(config.clone());
    let metrics = tester.run().await?;

    metrics.print_summary();

    // Generate reports
    if let Some(ref path) = cli.output_json {
        println!("\nGenerating JSON report: {}", path.display());
        Reporter::generate_json(&metrics, path)?;
    }

    if let Some(ref path) = cli.output_html {
        println!("Generating HTML report: {}", path.display());
        Reporter::generate_html(&metrics, path)?;
    }

    if let Some(ref path) = cli.output_csv {
        println!("Generating CSV report: {}", path.display());
        Reporter::generate_csv(&metrics, path)?;
    }

    if cli.prometheus {
        println!("\nPrometheus metrics:");
        println!("{}", Reporter::generate_prometheus_metrics(&metrics));
    }

    Ok(())
}
