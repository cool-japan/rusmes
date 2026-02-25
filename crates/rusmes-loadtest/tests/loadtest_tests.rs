//! Tests for rusmes-loadtest

use rusmes_loadtest::scenarios::ScenarioType;
use rusmes_loadtest::*;

#[test]
fn test_config_validation() {
    let config = LoadTestConfig::default();
    assert!(config.validate().is_ok());
}

#[test]
fn test_invalid_config_empty_host() {
    let config = LoadTestConfig {
        target_host: "".to_string(),
        ..LoadTestConfig::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn test_invalid_config_zero_port() {
    let config = LoadTestConfig {
        target_port: 0,
        ..LoadTestConfig::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn test_loadtester_creation() {
    let config = LoadTestConfig::default();
    let _tester = LoadTester::new(config);
}

#[tokio::test]
async fn test_loadtester_metrics() {
    let config = LoadTestConfig::default();
    let tester = LoadTester::new(config);
    let metrics = tester.get_metrics().await;
    assert_eq!(metrics.total_requests, 0);
}

#[test]
fn test_scenario_types() {
    let config = LoadTestConfig::default();
    let _runner = ScenarioType::SmtpThroughput.create_runner(&config);
    let _runner = ScenarioType::ConcurrentConnections.create_runner(&config);
    let _runner = ScenarioType::MixedProtocol.create_runner(&config);
    let _runner = ScenarioType::SustainedLoad.create_runner(&config);
}

#[test]
fn test_metrics_success_rate() {
    use std::time::Duration;

    let mut metrics = LoadTestMetrics::new();
    metrics.record_success(Duration::from_millis(10), 100, 50);
    metrics.record_success(Duration::from_millis(20), 100, 50);
    metrics.record_failure("Test error".to_string());

    assert_eq!(metrics.total_requests, 3);
    assert_eq!(metrics.successful_requests, 2);
    assert_eq!(metrics.failed_requests, 1);
    assert!((metrics.success_rate() - 0.6667).abs() < 0.001);
}

#[test]
fn test_metrics_latency_stats() {
    use std::time::Duration;

    let mut metrics = LoadTestMetrics::new();
    metrics.record_success(Duration::from_millis(10), 100, 50);
    metrics.record_success(Duration::from_millis(20), 100, 50);
    metrics.record_success(Duration::from_millis(30), 100, 50);

    let stats = metrics.latency_stats();
    assert!(stats.min.as_millis() >= 10);
    assert!(stats.max.as_millis() >= 30);
}

#[test]
fn test_message_generator() {
    use rusmes_loadtest::generators::MessageGenerator;

    let generator = MessageGenerator::new(1024, 2048);
    let message = generator.generate();

    assert!(message.len() >= 1024);
    assert!(message.len() <= 2048);
    assert!(message.contains("From:"));
    assert!(message.contains("To:"));
}

#[test]
fn test_message_with_attachment() {
    use rusmes_loadtest::generators::MessageGenerator;

    let generator = MessageGenerator::new(1024, 2048);
    let message = generator.generate_with_attachment(100);

    assert!(message.contains("MIME-Version"));
    assert!(message.contains("multipart/mixed"));
}

#[test]
fn test_config_serialization() {
    let config = LoadTestConfig::default();
    let json = serde_json::to_string(&config).unwrap();
    let deserialized: LoadTestConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(config.target_host, deserialized.target_host);
    assert_eq!(config.target_port, deserialized.target_port);
}

#[test]
fn test_latency_stats_default() {
    let stats = LatencyStats::default();
    assert_eq!(stats.min.as_millis(), 0);
    assert_eq!(stats.max.as_millis(), 0);
}

#[test]
fn test_metrics_duration() {
    let mut metrics = LoadTestMetrics::new();
    assert!(metrics.duration().is_none());

    metrics.mark_started();
    assert!(metrics.duration().is_some());

    metrics.mark_completed();
    let _duration = metrics.duration().unwrap();
    // Duration is always non-negative, no need to assert
}

#[test]
fn test_message_size_configuration() {
    use rusmes_loadtest::config::MessageSize;

    let fixed = MessageSize::Fixed(1024);
    assert_eq!(fixed.get(), (1024, 1024));

    let random = MessageSize::Random {
        min: 512,
        max: 2048,
    };
    assert_eq!(random.get(), (512, 2048));
}

#[test]
fn test_protocol_configuration() {
    use rusmes_loadtest::config::Protocol;

    let config = LoadTestConfig {
        protocol: Protocol::Mixed,
        mixed_weights: Some((70, 20, 10, 0)),
        ..LoadTestConfig::default()
    };

    assert!(config.validate().is_ok());
}

#[test]
fn test_invalid_mixed_protocol() {
    use rusmes_loadtest::config::Protocol;

    let config = LoadTestConfig {
        protocol: Protocol::Mixed,
        mixed_weights: None,
        ..LoadTestConfig::default()
    };

    assert!(config.validate().is_err());
}

#[test]
fn test_zero_weights_invalid() {
    use rusmes_loadtest::config::Protocol;

    let config = LoadTestConfig {
        protocol: Protocol::Mixed,
        mixed_weights: Some((0, 0, 0, 0)),
        ..LoadTestConfig::default()
    };

    assert!(config.validate().is_err());
}

#[test]
fn test_workload_patterns() {
    use rusmes_loadtest::workload::WorkloadPattern;
    use std::time::Duration;

    let steady = WorkloadPattern::Steady { rate: 1000 };
    assert_eq!(steady.rate_at(Duration::from_secs(0)), 1000);
    assert_eq!(steady.rate_at(Duration::from_secs(100)), 1000);

    let ramp = WorkloadPattern::RampUp {
        start_rate: 100,
        end_rate: 1000,
        duration: Duration::from_secs(10),
    };
    assert_eq!(ramp.rate_at(Duration::from_secs(0)), 100);
    assert_eq!(ramp.rate_at(Duration::from_secs(10)), 1000);
}
