//! AmateRS cluster configuration types.

/// Consistency level for operations.
///
/// `Default` is [`ConsistencyLevel::Quorum`], matching the [`AmatersConfig`]
/// defaults for both read and write consistency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConsistencyLevel {
    /// Require all replicas
    All,
    /// Require quorum of replicas (default)
    #[default]
    Quorum,
    /// Require only one replica
    One,
    /// Local quorum (same datacenter)
    LocalQuorum,
}

/// AmateRS cluster configuration.
///
/// ## Forward-compatibility notes (amaters-sdk-rust v0.2.0)
///
/// The fields `replication_factor`, `read_consistency`, and `write_consistency`
/// are accepted here for configuration-file compatibility and future SDK
/// releases, but the current amaters-sdk-rust v0.2.0 does not expose these
/// settings at connect time.  Non-default values will emit a `tracing::warn!`
/// at startup so operators know the field is being silently ignored.
///
/// The fields `max_retries`, `initial_backoff_ms`, and `max_backoff_ms` are
/// wired directly into the SDK's `RetryConfig` and take effect immediately.
#[derive(Debug, Clone)]
pub struct AmatersConfig {
    /// Cluster contact points (host:port)
    pub cluster_endpoints: Vec<String>,
    /// Keyspace for metadata
    pub metadata_keyspace: String,
    /// Keyspace for message blobs
    pub blob_keyspace: String,
    /// Replication factor (default: [`AmatersConfig::DEFAULT_REPLICATION_FACTOR`]).
    ///
    /// **Accepted for forward-compatibility only.**  amaters-sdk-rust v0.2.0 does
    /// not expose replication_factor via its connect API; the cluster-side setting
    /// governs replication.  A non-default value emits a startup warning.
    pub replication_factor: usize,
    /// Consistency level for reads.
    ///
    /// **Accepted for forward-compatibility only.**  amaters-sdk-rust v0.2.0 has
    /// no per-call or per-connection consistency knob.  A non-default value emits
    /// a startup warning.
    pub read_consistency: ConsistencyLevel,
    /// Consistency level for writes.
    ///
    /// **Accepted for forward-compatibility only.**  amaters-sdk-rust v0.2.0 has
    /// no per-call or per-connection consistency knob.  A non-default value emits
    /// a startup warning.
    pub write_consistency: ConsistencyLevel,
    /// Connection timeout in milliseconds
    pub timeout_ms: u64,
    /// Maximum retry attempts wired into the SDK `RetryConfig`. Default: 3.
    pub max_retries: usize,
    /// Initial backoff for retry in milliseconds. Default: 100 ms.
    pub initial_backoff_ms: u64,
    /// Maximum backoff cap for retry in milliseconds. Default: 5 000 ms.
    pub max_backoff_ms: u64,
    /// Enable compression
    pub enable_compression: bool,
    /// Circuit breaker failure threshold
    pub circuit_breaker_threshold: usize,
    /// Circuit breaker timeout in milliseconds
    pub circuit_breaker_timeout_ms: u64,
}

impl AmatersConfig {
    /// Default replication factor applied when the config is not overridden.
    ///
    /// amaters-sdk-rust v0.2.0 does not expose replication_factor at connect
    /// time; this constant is used solely for the non-default-value detection
    /// that triggers a startup warning.
    pub const DEFAULT_REPLICATION_FACTOR: usize = 3;

    /// Parse an AmateRS connection URL into a config.
    ///
    /// Accepted format:
    /// `amaters://host1:port,host2:port/keyspace?max_retries=5&initial_backoff_ms=200&max_backoff_ms=10000`
    ///
    /// - The scheme must be `amaters`.
    /// - The authority contains one or more comma-separated `host:port` endpoints.
    /// - The first path segment (after the leading `/`) becomes both the
    ///   `metadata_keyspace` and the base name for the `blob_keyspace`
    ///   (`<keyspace>_blobs`).  If no path is given the defaults
    ///   (`rusmes_metadata` / `rusmes_blobs`) are used.
    /// - Optional query parameters: `max_retries`, `initial_backoff_ms`,
    ///   `max_backoff_ms` (all parsed as unsigned integers).
    ///
    /// All other fields take their [`Default`] values.
    ///
    /// # Errors
    ///
    /// Returns an error if the scheme is not `amaters`, the host list is empty,
    /// any host:port entry is malformed, or a query parameter value cannot be
    /// parsed as an unsigned integer.
    pub fn from_url(url: &str) -> anyhow::Result<Self> {
        // Require the amaters:// scheme prefix.
        let rest = url.strip_prefix("amaters://").ok_or_else(|| {
            anyhow::anyhow!("AmateRS URL must start with 'amaters://', got: {url}")
        })?;

        // Split the query string off first (everything after '?').
        let (rest_no_query, query_string) = match rest.find('?') {
            Some(idx) => (&rest[..idx], &rest[idx + 1..]),
            None => (rest, ""),
        };

        // Split authority from optional path.
        let (authority, path_segment) = match rest_no_query.find('/') {
            Some(idx) => (&rest_no_query[..idx], &rest_no_query[idx + 1..]),
            None => (rest_no_query, ""),
        };

        if authority.is_empty() {
            return Err(anyhow::anyhow!(
                "AmateRS URL contains no host endpoints: {url}"
            ));
        }

        // Parse each comma-separated endpoint and validate host:port form.
        let cluster_endpoints: Vec<String> = authority
            .split(',')
            .map(|ep| {
                let ep = ep.trim();
                if ep.is_empty() {
                    return Err(anyhow::anyhow!("Empty endpoint in AmateRS URL: {url}"));
                }
                // Validate that there is a port component.
                if !ep.contains(':') {
                    return Err(anyhow::anyhow!(
                        "AmateRS endpoint '{ep}' is missing a port (expected host:port)"
                    ));
                }
                Ok(ep.to_string())
            })
            .collect::<anyhow::Result<Vec<String>>>()?;

        if cluster_endpoints.is_empty() {
            return Err(anyhow::anyhow!(
                "AmateRS URL contains no valid endpoints: {url}"
            ));
        }

        // Derive keyspace names from the path segment when present.
        let keyspace = path_segment.trim_matches('/');
        let (metadata_keyspace, blob_keyspace) = if keyspace.is_empty() {
            ("rusmes_metadata".to_string(), "rusmes_blobs".to_string())
        } else {
            (keyspace.to_string(), format!("{keyspace}_blobs"))
        };

        // Start with defaults then overlay query-string overrides.
        let mut cfg = Self {
            cluster_endpoints,
            metadata_keyspace,
            blob_keyspace,
            ..Self::default()
        };

        // Parse optional query parameters.
        if !query_string.is_empty() {
            for pair in query_string.split('&') {
                let mut parts = pair.splitn(2, '=');
                let key = parts.next().unwrap_or("").trim();
                let val = parts.next().unwrap_or("").trim();
                match key {
                    "max_retries" => {
                        cfg.max_retries = val.parse::<usize>().map_err(|_| {
                            anyhow::anyhow!(
                                "AmateRS URL: invalid max_retries value '{val}': expected unsigned integer"
                            )
                        })?;
                    }
                    "initial_backoff_ms" => {
                        cfg.initial_backoff_ms = val.parse::<u64>().map_err(|_| {
                            anyhow::anyhow!(
                                "AmateRS URL: invalid initial_backoff_ms value '{val}': expected unsigned integer"
                            )
                        })?;
                    }
                    "max_backoff_ms" => {
                        cfg.max_backoff_ms = val.parse::<u64>().map_err(|_| {
                            anyhow::anyhow!(
                                "AmateRS URL: invalid max_backoff_ms value '{val}': expected unsigned integer"
                            )
                        })?;
                    }
                    // Unknown query params are silently ignored to allow forward-compat.
                    _ => {}
                }
            }
        }

        Ok(cfg)
    }
}

impl Default for AmatersConfig {
    fn default() -> Self {
        Self {
            cluster_endpoints: vec!["localhost:9042".to_string()],
            metadata_keyspace: "rusmes_metadata".to_string(),
            blob_keyspace: "rusmes_blobs".to_string(),
            replication_factor: Self::DEFAULT_REPLICATION_FACTOR,
            read_consistency: ConsistencyLevel::default(),
            write_consistency: ConsistencyLevel::default(),
            timeout_ms: 10_000,
            max_retries: 3,
            initial_backoff_ms: 100,
            max_backoff_ms: 5_000,
            enable_compression: true,
            circuit_breaker_threshold: 5,
            circuit_breaker_timeout_ms: 60_000,
        }
    }
}
