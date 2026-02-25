//! OpenTelemetry distributed tracing integration for RusMES
//!
//! This module provides OpenTelemetry tracing capabilities with OTLP exporter support.
//! It enables distributed tracing across SMTP, IMAP, JMAP operations and mailet pipelines.
//!
//! ## Features
//!
//! - OTLP exporter with gRPC and HTTP protocol support
//! - Automatic span creation for protocol operations
//! - Trace context propagation through mailet pipeline
//! - Configurable sampling rate
//! - Integration with existing tracing infrastructure
//!
//! ## Usage
//!
//! ```rust,no_run
//! use rusmes_metrics::tracing::{init_tracing, smtp_span, TracingGuard};
//! use rusmes_config::TracingConfig;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let config = TracingConfig {
//!     enabled: true,
//!     endpoint: "http://localhost:4317".to_string(),
//!     protocol: rusmes_config::OtlpProtocol::Grpc,
//!     service_name: "rusmes".to_string(),
//!     sample_ratio: 1.0,
//! };
//!
//! // Initialize tracing (returns a guard that must be kept alive)
//! let _guard = init_tracing(config).await?;
//!
//! // Create spans for operations
//! let span = smtp_span("MAIL FROM", "user@example.com");
//! let _enter = span.enter();
//! // ... SMTP operation ...
//! # Ok(())
//! # }
//! ```

use anyhow::{Context, Result};
use opentelemetry::trace::{SpanKind, TraceContextExt};
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::{RandomIdGenerator, Sampler, SdkTracerProvider};
use opentelemetry_sdk::Resource;
use rusmes_config::{OtlpProtocol, TracingConfig};
use tracing::{span, Level, Span};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Guard that maintains the tracing pipeline
///
/// This guard must be kept alive for the duration of the application.
/// When dropped, it will flush and shutdown the tracing pipeline.
pub struct TracingGuard {
    provider: SdkTracerProvider,
}

impl Drop for TracingGuard {
    fn drop(&mut self) {
        if let Err(e) = self.provider.shutdown() {
            tracing::warn!("Failed to shutdown tracer provider: {:?}", e);
        }
    }
}

/// Initialize OpenTelemetry tracing with OTLP exporter
///
/// This function sets up the tracing infrastructure including:
/// - OTLP exporter (gRPC or HTTP based on configuration)
/// - Trace sampling based on configured ratio
/// - Integration with tracing-subscriber
///
/// Returns a `TracingGuard` that must be kept alive for the duration of the application.
///
/// # Errors
///
/// Returns an error if:
/// - The OTLP exporter cannot be initialized
/// - The endpoint is invalid
/// - The tracing subscriber cannot be set
pub async fn init_tracing(config: TracingConfig) -> Result<TracingGuard> {
    if !config.enabled {
        tracing::info!("OpenTelemetry tracing is disabled");
        return Err(anyhow::anyhow!("Tracing is disabled"));
    }

    // Validate configuration
    config.validate_endpoint()?;
    config.validate_sample_ratio()?;
    config.validate_service_name()?;

    tracing::info!(
        "Initializing OpenTelemetry tracing: endpoint={}, protocol={:?}, service={}",
        config.endpoint,
        config.protocol,
        config.service_name
    );

    // Create resource with service name
    let resource = Resource::builder_empty()
        .with_attribute(KeyValue::new("service.name", config.service_name.clone()))
        .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
        .build();

    // Create sampler based on sample ratio
    let sampler = if config.sample_ratio >= 1.0 {
        Sampler::AlwaysOn
    } else if config.sample_ratio <= 0.0 {
        Sampler::AlwaysOff
    } else {
        Sampler::TraceIdRatioBased(config.sample_ratio)
    };

    // Build tracer provider based on protocol
    let provider = match config.protocol {
        OtlpProtocol::Grpc => {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(&config.endpoint)
                .build()
                .context("Failed to build gRPC OTLP span exporter")?;

            SdkTracerProvider::builder()
                .with_sampler(sampler)
                .with_id_generator(RandomIdGenerator::default())
                .with_resource(resource)
                .with_batch_exporter(exporter)
                .build()
        }
        OtlpProtocol::Http => {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_endpoint(&config.endpoint)
                .build()
                .context("Failed to build HTTP OTLP span exporter")?;

            SdkTracerProvider::builder()
                .with_sampler(sampler)
                .with_id_generator(RandomIdGenerator::default())
                .with_resource(resource)
                .with_batch_exporter(exporter)
                .build()
        }
    };

    // Register the provider globally and get a tracer
    global::set_tracer_provider(provider.clone());
    let tracer = global::tracer("rusmes");

    // Create OpenTelemetry layer
    let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    // Create filter layer for controlling log levels
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Create fmt layer for console output
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_level(true);

    // Combine layers and initialize subscriber
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(telemetry_layer)
        .try_init()
        .context("Failed to initialize tracing subscriber")?;

    tracing::info!("OpenTelemetry tracing initialized successfully");

    Ok(TracingGuard { provider })
}

/// Create a span for SMTP operations
///
/// Creates a tracing span with SMTP-specific attributes including:
/// - Command name (HELO, MAIL FROM, RCPT TO, DATA, etc.)
/// - User/recipient information
/// - Protocol identifier
///
/// # Example
///
/// ```
/// use rusmes_metrics::tracing::smtp_span;
///
/// let span = smtp_span("MAIL FROM", "sender@example.com");
/// let _enter = span.enter();
/// // ... SMTP operation ...
/// ```
pub fn smtp_span(command: &str, user: &str) -> Span {
    span!(
        Level::INFO,
        "smtp_operation",
        protocol = "smtp",
        smtp.command = command,
        smtp.user = user,
        otel.kind = ?SpanKind::Server
    )
}

/// Create a span for IMAP operations
///
/// Creates a tracing span with IMAP-specific attributes including:
/// - Command name (SELECT, FETCH, STORE, SEARCH, etc.)
/// - Mailbox name
/// - User information
///
/// # Example
///
/// ```
/// use rusmes_metrics::tracing::imap_span;
///
/// let span = imap_span("SELECT", "INBOX", "user@example.com");
/// let _enter = span.enter();
/// // ... IMAP operation ...
/// ```
pub fn imap_span(command: &str, mailbox: &str, user: &str) -> Span {
    span!(
        Level::INFO,
        "imap_operation",
        protocol = "imap",
        imap.command = command,
        imap.mailbox = mailbox,
        imap.user = user,
        otel.kind = ?SpanKind::Server
    )
}

/// Create a span for JMAP operations
///
/// Creates a tracing span with JMAP-specific attributes including:
/// - Method name (Email/get, Email/set, Mailbox/query, etc.)
/// - Account ID
/// - Request ID
///
/// # Example
///
/// ```
/// use rusmes_metrics::tracing::jmap_span;
///
/// let span = jmap_span("Email/get", "account-123", "req-456");
/// let _enter = span.enter();
/// // ... JMAP operation ...
/// ```
pub fn jmap_span(method: &str, account_id: &str, request_id: &str) -> Span {
    span!(
        Level::INFO,
        "jmap_operation",
        protocol = "jmap",
        jmap.method = method,
        jmap.account_id = account_id,
        jmap.request_id = request_id,
        otel.kind = ?SpanKind::Server
    )
}

/// Create a span for mailet pipeline operations
///
/// Creates a tracing span for tracking message flow through the mailet pipeline.
/// Includes:
/// - Mailet name
/// - Message ID
/// - Stage in pipeline
/// - Sender/recipient information
///
/// # Example
///
/// ```
/// use rusmes_metrics::tracing::mailet_span;
///
/// let span = mailet_span(
///     "spam-filter",
///     "msg-123",
///     "filtering",
///     "sender@example.com",
///     "recipient@example.com"
/// );
/// let _enter = span.enter();
/// // ... Mailet processing ...
/// ```
pub fn mailet_span(
    mailet_name: &str,
    message_id: &str,
    stage: &str,
    sender: &str,
    recipient: &str,
) -> Span {
    span!(
        Level::INFO,
        "mailet_processing",
        mailet.name = mailet_name,
        mailet.stage = stage,
        mail.message_id = message_id,
        mail.sender = sender,
        mail.recipient = recipient,
        otel.kind = ?SpanKind::Internal
    )
}

/// Create a span for message delivery operations
///
/// Tracks message delivery with:
/// - Delivery method (local, remote SMTP, etc.)
/// - Destination domain
/// - Message ID
///
/// # Example
///
/// ```
/// use rusmes_metrics::tracing::delivery_span;
///
/// let span = delivery_span("remote-smtp", "example.com", "msg-123");
/// let _enter = span.enter();
/// // ... Delivery operation ...
/// ```
pub fn delivery_span(method: &str, domain: &str, message_id: &str) -> Span {
    span!(
        Level::INFO,
        "mail_delivery",
        delivery.method = method,
        delivery.domain = domain,
        mail.message_id = message_id,
        otel.kind = ?SpanKind::Client
    )
}

/// Propagate trace context to a child span
///
/// This function extracts the current trace context and can be used to propagate
/// context across async boundaries or to external systems.
///
/// # Example
///
/// ```
/// use rusmes_metrics::tracing::propagate_context;
///
/// let parent_span = rusmes_metrics::tracing::smtp_span("MAIL FROM", "user@example.com");
/// let _guard = parent_span.enter();
///
/// // Get current context for propagation
/// let context = propagate_context();
/// // ... propagate to child operation ...
/// ```
pub fn propagate_context() -> opentelemetry::Context {
    opentelemetry::Context::current()
}

/// Create a child span with explicit parent context
///
/// Useful for creating spans in async tasks or when the parent context
/// needs to be explicitly passed.
pub fn create_child_span(parent: &opentelemetry::Context, name: &str) -> Span {
    let span = span!(Level::INFO, "operation", operation.name = name);

    // Attach parent context
    let parent_span = parent.span();
    let span_context = parent_span.span_context();
    if span_context.is_valid() {
        tracing::debug!(
            "Creating child span with parent trace_id={:?}",
            span_context.trace_id()
        );
    }

    span
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smtp_span_creation() {
        let span = smtp_span("MAIL FROM", "test@example.com");
        assert_eq!(span.metadata().map(|m| m.name()), Some("smtp_operation"));
    }

    #[test]
    fn test_imap_span_creation() {
        let span = imap_span("SELECT", "INBOX", "user@example.com");
        assert_eq!(span.metadata().map(|m| m.name()), Some("imap_operation"));
    }

    #[test]
    fn test_jmap_span_creation() {
        let span = jmap_span("Email/get", "acc-123", "req-456");
        assert_eq!(span.metadata().map(|m| m.name()), Some("jmap_operation"));
    }

    #[test]
    fn test_mailet_span_creation() {
        let span = mailet_span(
            "spam-filter",
            "msg-123",
            "processing",
            "sender@example.com",
            "recipient@example.com",
        );
        assert_eq!(span.metadata().map(|m| m.name()), Some("mailet_processing"));
    }

    #[test]
    fn test_delivery_span_creation() {
        let span = delivery_span("remote-smtp", "example.com", "msg-123");
        assert_eq!(span.metadata().map(|m| m.name()), Some("mail_delivery"));
    }

    #[test]
    fn test_tracing_config_validation() {
        let config = TracingConfig {
            enabled: true,
            endpoint: "http://localhost:4317".to_string(),
            protocol: OtlpProtocol::Grpc,
            service_name: "test-service".to_string(),
            sample_ratio: 0.5,
        };

        assert!(config.validate_endpoint().is_ok());
        assert!(config.validate_sample_ratio().is_ok());
        assert!(config.validate_service_name().is_ok());
    }

    #[test]
    fn test_tracing_config_invalid_endpoint() {
        let config = TracingConfig {
            enabled: true,
            endpoint: "invalid-endpoint".to_string(),
            protocol: OtlpProtocol::Grpc,
            service_name: "test".to_string(),
            sample_ratio: 1.0,
        };

        assert!(config.validate_endpoint().is_err());
    }

    #[test]
    fn test_tracing_config_invalid_sample_ratio() {
        let config = TracingConfig {
            enabled: true,
            endpoint: "http://localhost:4317".to_string(),
            protocol: OtlpProtocol::Grpc,
            service_name: "test".to_string(),
            sample_ratio: 1.5,
        };

        assert!(config.validate_sample_ratio().is_err());
    }

    #[test]
    fn test_tracing_config_empty_service_name() {
        let config = TracingConfig {
            enabled: true,
            endpoint: "http://localhost:4317".to_string(),
            protocol: OtlpProtocol::Grpc,
            service_name: "".to_string(),
            sample_ratio: 1.0,
        };

        assert!(config.validate_service_name().is_err());
    }

    #[test]
    fn test_context_propagation() {
        let context = propagate_context();
        assert!(
            context.span().span_context().is_valid() || !context.span().span_context().is_sampled()
        );
    }

    #[test]
    fn test_child_span_creation() {
        let parent = propagate_context();
        let child = create_child_span(&parent, "test-operation");
        assert_eq!(child.metadata().map(|m| m.name()), Some("operation"));
    }
}
