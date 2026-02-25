//! Load test scenarios

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::sleep;

use crate::config::LoadTestConfig;
use crate::generators::MessageGenerator;
use crate::metrics::LoadTestMetrics;
use crate::protocols::{ImapClient, SmtpClient};

/// Available test scenarios
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ScenarioType {
    /// SMTP throughput test
    SmtpThroughput,
    /// Concurrent connections test
    ConcurrentConnections,
    /// Mixed protocol test
    MixedProtocol,
    /// Sustained load test
    SustainedLoad,
}

impl ScenarioType {
    /// Create a scenario runner
    pub fn create_runner(
        &self,
        config: &LoadTestConfig,
    ) -> Box<dyn LoadTestScenario + Send + Sync> {
        match self {
            ScenarioType::SmtpThroughput => Box::new(SmtpThroughputScenario::new(config.clone())),
            ScenarioType::ConcurrentConnections => {
                Box::new(ConcurrentConnectionsScenario::new(config.clone()))
            }
            ScenarioType::MixedProtocol => Box::new(MixedProtocolScenario::new(config.clone())),
            ScenarioType::SustainedLoad => Box::new(SustainedLoadScenario::new(config.clone())),
        }
    }
}

/// Trait for load test scenarios
pub trait LoadTestScenario {
    /// Run the scenario
    fn run(
        &self,
        metrics: Arc<RwLock<LoadTestMetrics>>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;
}

/// SMTP throughput scenario
pub struct SmtpThroughputScenario {
    config: LoadTestConfig,
}

impl SmtpThroughputScenario {
    pub fn new(config: LoadTestConfig) -> Self {
        Self { config }
    }
}

impl LoadTestScenario for SmtpThroughputScenario {
    fn run(
        &self,
        metrics: Arc<RwLock<LoadTestMetrics>>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            metrics.write().await.mark_started();

            let (min_size, max_size) = self.config.message_size_range();
            let generator = MessageGenerator::with_content_type(
                min_size,
                max_size,
                self.config.message_content,
            );

            let duration = Duration::from_secs(self.config.duration_secs);
            let start = Instant::now();

            let mut tasks = vec![];

            for _ in 0..self.config.concurrency {
                let metrics = metrics.clone();
                let config = self.config.clone();
                let generator = generator.clone();

                let task = tokio::spawn(async move {
                    while start.elapsed() < duration {
                        let message = generator.generate();
                        let request_start = Instant::now();

                        match SmtpClient::send_message(
                            &config.target_host,
                            config.target_port,
                            &message,
                        )
                        .await
                        {
                            Ok(bytes_received) => {
                                let latency = request_start.elapsed();
                                metrics.write().await.record_success(
                                    latency,
                                    message.len(),
                                    bytes_received,
                                );
                            }
                            Err(e) => {
                                metrics.write().await.record_failure(e.to_string());
                            }
                        }

                        // Rate limiting
                        let delay = Duration::from_millis(1000 / config.message_rate);
                        sleep(delay).await;
                    }
                });

                tasks.push(task);
            }

            for task in tasks {
                let _ = task.await;
            }

            metrics.write().await.mark_completed();
            Ok(())
        })
    }
}

/// Concurrent connections scenario
pub struct ConcurrentConnectionsScenario {
    config: LoadTestConfig,
}

impl ConcurrentConnectionsScenario {
    pub fn new(config: LoadTestConfig) -> Self {
        Self { config }
    }
}

impl LoadTestScenario for ConcurrentConnectionsScenario {
    fn run(
        &self,
        metrics: Arc<RwLock<LoadTestMetrics>>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            metrics.write().await.mark_started();

            let (min_size, max_size) = self.config.message_size_range();
            let generator = MessageGenerator::with_content_type(
                min_size,
                max_size,
                self.config.message_content,
            );

            let mut tasks = vec![];

            // Create many concurrent connections
            for _ in 0..self.config.concurrency {
                let metrics = metrics.clone();
                let config = self.config.clone();
                let generator = generator.clone();

                let task = tokio::spawn(async move {
                    let message = generator.generate();
                    let request_start = Instant::now();

                    match SmtpClient::send_message(
                        &config.target_host,
                        config.target_port,
                        &message,
                    )
                    .await
                    {
                        Ok(bytes_received) => {
                            let latency = request_start.elapsed();
                            metrics.write().await.record_success(
                                latency,
                                message.len(),
                                bytes_received,
                            );
                        }
                        Err(e) => {
                            metrics.write().await.record_failure(e.to_string());
                        }
                    }
                });

                tasks.push(task);
            }

            for task in tasks {
                let _ = task.await;
            }

            metrics.write().await.mark_completed();
            Ok(())
        })
    }
}

/// Mixed protocol scenario
pub struct MixedProtocolScenario {
    config: LoadTestConfig,
}

impl MixedProtocolScenario {
    pub fn new(config: LoadTestConfig) -> Self {
        Self { config }
    }
}

impl LoadTestScenario for MixedProtocolScenario {
    fn run(
        &self,
        metrics: Arc<RwLock<LoadTestMetrics>>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            metrics.write().await.mark_started();

            let (min_size, max_size) = self.config.message_size_range();
            let generator = MessageGenerator::with_content_type(
                min_size,
                max_size,
                self.config.message_content,
            );

            let duration = Duration::from_secs(self.config.duration_secs);
            let start = Instant::now();

            let mut tasks = vec![];

            for i in 0..self.config.concurrency {
                let metrics = metrics.clone();
                let config = self.config.clone();
                let generator = generator.clone();

                let task = tokio::spawn(async move {
                    while start.elapsed() < duration {
                        if i % 2 == 0 {
                            // SMTP
                            let message = generator.generate();
                            let request_start = Instant::now();

                            match SmtpClient::send_message(
                                &config.target_host,
                                config.target_port,
                                &message,
                            )
                            .await
                            {
                                Ok(bytes_received) => {
                                    let latency = request_start.elapsed();
                                    metrics.write().await.record_success(
                                        latency,
                                        message.len(),
                                        bytes_received,
                                    );
                                }
                                Err(e) => {
                                    metrics.write().await.record_failure(e.to_string());
                                }
                            }
                        } else {
                            // IMAP
                            let request_start = Instant::now();

                            match ImapClient::fetch_messages(
                                &config.target_host,
                                config.target_port + 100,
                            )
                            .await
                            {
                                Ok(bytes_received) => {
                                    let latency = request_start.elapsed();
                                    metrics.write().await.record_success(
                                        latency,
                                        0,
                                        bytes_received,
                                    );
                                }
                                Err(e) => {
                                    metrics.write().await.record_failure(e.to_string());
                                }
                            }
                        }

                        sleep(Duration::from_millis(100)).await;
                    }
                });

                tasks.push(task);
            }

            for task in tasks {
                let _ = task.await;
            }

            metrics.write().await.mark_completed();
            Ok(())
        })
    }
}

/// Sustained load scenario
pub struct SustainedLoadScenario {
    config: LoadTestConfig,
}

impl SustainedLoadScenario {
    pub fn new(config: LoadTestConfig) -> Self {
        Self { config }
    }
}

impl LoadTestScenario for SustainedLoadScenario {
    fn run(
        &self,
        metrics: Arc<RwLock<LoadTestMetrics>>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            metrics.write().await.mark_started();

            let (min_size, max_size) = self.config.message_size_range();
            let generator = MessageGenerator::with_content_type(
                min_size,
                max_size,
                self.config.message_content,
            );

            let duration = Duration::from_secs(self.config.duration_secs);
            let start = Instant::now();

            let mut tasks = vec![];

            for _ in 0..self.config.concurrency {
                let metrics = metrics.clone();
                let config = self.config.clone();
                let generator = generator.clone();

                let task = tokio::spawn(async move {
                    while start.elapsed() < duration {
                        let message = generator.generate();
                        let request_start = Instant::now();

                        match SmtpClient::send_message(
                            &config.target_host,
                            config.target_port,
                            &message,
                        )
                        .await
                        {
                            Ok(bytes_received) => {
                                let latency = request_start.elapsed();
                                metrics.write().await.record_success(
                                    latency,
                                    message.len(),
                                    bytes_received,
                                );
                            }
                            Err(e) => {
                                metrics.write().await.record_failure(e.to_string());
                            }
                        }

                        // Constant rate
                        let delay = Duration::from_millis(1000);
                        sleep(delay).await;
                    }
                });

                tasks.push(task);
            }

            for task in tasks {
                let _ = task.await;
            }

            metrics.write().await.mark_completed();
            Ok(())
        })
    }
}

pub type ScenarioRunner = Box<dyn LoadTestScenario + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scenario_creation() {
        let config = LoadTestConfig::default();
        let _runner = ScenarioType::SmtpThroughput.create_runner(&config);
        let _runner = ScenarioType::ConcurrentConnections.create_runner(&config);
        let _runner = ScenarioType::MixedProtocol.create_runner(&config);
        let _runner = ScenarioType::SustainedLoad.create_runner(&config);
    }
}
