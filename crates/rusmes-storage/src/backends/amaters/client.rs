//! AmateRS client — dispatches between mock (in-memory) and real SDK variants.

use super::circuit_breaker::CircuitBreaker;
use super::config::AmatersConfig;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// Real AmateRS SDK — only compiled when the `amaters-backend` feature is enabled.
// The SDK has C transitive deps (aws-lc-rs, ring) via tonic/rustls; keep opt-in.
#[cfg(feature = "amaters-backend")]
use amaters_sdk_rust::{
    AmateRSClient, CipherBlob as SdkCipherBlob, ClientConfig as SdkClientConfig, Key as SdkKey,
    RetryConfig as SdkRetryConfig,
};

// ---------------------------------------------------------------------------
// Scheme helper (real client only)
// ---------------------------------------------------------------------------

/// Prepend `http://` if no scheme is present in the endpoint string.
#[cfg(feature = "amaters-backend")]
fn ensure_scheme(endpoint: &str) -> String {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.to_owned()
    } else {
        format!("http://{endpoint}")
    }
}

// ---------------------------------------------------------------------------
// Prefix upper-bound helper (real client only)
// ---------------------------------------------------------------------------

/// Compute an exclusive upper bound for a lexicographic prefix range scan.
///
/// Increments the last non-`0xFF` byte of `prefix`.  Returns `None` when all
/// bytes are `0xFF` (very unlikely for human-readable keys).
#[cfg(feature = "amaters-backend")]
fn prefix_upper_bound(prefix: &str) -> Option<Vec<u8>> {
    let mut upper = prefix.as_bytes().to_vec();
    for byte in upper.iter_mut().rev() {
        if *byte < 0xFF {
            *byte += 1;
            return Some(upper);
        }
        *byte = 0x00;
    }
    None
}

// ---------------------------------------------------------------------------
// Inner state for the mock path
// ---------------------------------------------------------------------------

/// State held by the in-memory mock variant of `AmatersClient`.
pub(super) struct MockState {
    pub(super) config: AmatersConfig,
    pub(super) metadata: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    pub(super) blobs: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    pub(super) circuit_breaker: CircuitBreaker,
}

// ---------------------------------------------------------------------------
// AmatersClient — enum dispatching between mock and real SDK client
// ---------------------------------------------------------------------------

/// AmateRS client.
///
/// - `Mock` variant: in-memory HashMap-backed implementation used for unit tests
///   and development builds without the `amaters-backend` feature.
/// - `Real` variant (requires `amaters-backend` feature): wraps the real
///   `amaters-sdk-rust v0.2` gRPC client connected to a live AmateRS cluster.
pub(super) enum AmatersClient {
    Mock(MockState),
    #[cfg(feature = "amaters-backend")]
    Real {
        sdk: Arc<AmateRSClient>,
        metadata_collection: String,
        blob_collection: String,
    },
}

impl AmatersClient {
    /// Create an in-memory mock client (no network, suitable for tests and dev builds).
    pub(super) fn new(config: AmatersConfig) -> Self {
        let circuit_breaker = CircuitBreaker::new(
            config.circuit_breaker_threshold,
            config.circuit_breaker_timeout_ms,
        );
        Self::Mock(MockState {
            config,
            metadata: Arc::new(RwLock::new(HashMap::new())),
            blobs: Arc::new(RwLock::new(HashMap::new())),
            circuit_breaker,
        })
    }

    /// Create a real AmateRS SDK client connected to the cluster.
    ///
    /// Requires the `amaters-backend` feature.  Uses the first endpoint from
    /// `config.cluster_endpoints` as the primary gRPC target; the SDK's own
    /// connection pool and retry logic handle failover for subsequent ops.
    ///
    /// Retry parameters (`max_retries`, `initial_backoff_ms`, `max_backoff_ms`)
    /// are wired directly into the SDK's [`SdkRetryConfig`].
    ///
    /// `replication_factor` and `read/write_consistency` are checked and, when
    /// non-default, emit a `tracing::warn!` because amaters-sdk-rust v0.2.0
    /// does not expose these settings at connect time.
    #[cfg(feature = "amaters-backend")]
    pub(super) async fn new_real(config: &AmatersConfig) -> anyhow::Result<Self> {
        use super::config::ConsistencyLevel;
        use std::time::Duration;

        // Build the SDK retry configuration from our settings.
        let retry_config = SdkRetryConfig {
            max_retries: config.max_retries,
            initial_backoff: Duration::from_millis(config.initial_backoff_ms),
            max_backoff: Duration::from_millis(config.max_backoff_ms),
            // Preserve SDK defaults for parameters not yet surfaced in AmatersConfig.
            backoff_multiplier: 2.0,
            jitter: true,
        };

        // Warn when forward-compat fields are set to non-default values —
        // amaters-sdk-rust v0.2.0 silently ignores them otherwise.
        if config.replication_factor != AmatersConfig::DEFAULT_REPLICATION_FACTOR {
            tracing::warn!(
                target: "rusmes::storage::amaters",
                configured = config.replication_factor,
                default = AmatersConfig::DEFAULT_REPLICATION_FACTOR,
                "amaters: replication_factor config field is set but will be ignored — \
                 amaters-sdk-rust v0.2.0 does not expose replication_factor via its connect API. \
                 The cluster-side replication factor governs replication for now."
            );
        }
        if config.read_consistency != ConsistencyLevel::default()
            || config.write_consistency != ConsistencyLevel::default()
        {
            tracing::warn!(
                target: "rusmes::storage::amaters",
                read_consistency = ?config.read_consistency,
                write_consistency = ?config.write_consistency,
                "amaters: read_consistency/write_consistency config fields are set but will be \
                 ignored — amaters-sdk-rust v0.2.0 has no per-call or per-connection consistency \
                 knob. All operations use the SDK's default consistency."
            );
        }

        if config.cluster_endpoints.is_empty() {
            return Err(anyhow::anyhow!("AmatersConfig has no cluster endpoints"));
        }

        let mut last_err: Option<anyhow::Error> = None;
        let mut connected_sdk: Option<AmateRSClient> = None;
        for endpoint in &config.cluster_endpoints {
            let addr = ensure_scheme(endpoint);
            let sdk_config = SdkClientConfig::new(addr)
                .with_connect_timeout(Duration::from_millis(config.timeout_ms))
                .with_request_timeout(Duration::from_millis(config.timeout_ms.saturating_mul(3)))
                .with_retry_config(retry_config.clone());
            match AmateRSClient::connect_with_config(sdk_config).await {
                Ok(client) => {
                    tracing::info!(
                        target: "rusmes::storage::amaters",
                        "amaters: connected via endpoint {}", endpoint
                    );
                    connected_sdk = Some(client);
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        target: "rusmes::storage::amaters",
                        endpoint = %endpoint,
                        error = %e,
                        "amaters: connect attempt failed; cycling to next endpoint"
                    );
                    last_err = Some(anyhow::anyhow!("connect to {endpoint} failed: {e}"));
                }
            }
        }
        let sdk = connected_sdk.ok_or_else(|| {
            last_err.unwrap_or_else(|| {
                anyhow::anyhow!(
                    "amaters: all {} cluster endpoints unreachable",
                    config.cluster_endpoints.len()
                )
            })
        })?;

        Ok(Self::Real {
            sdk: Arc::new(sdk),
            metadata_collection: config.metadata_keyspace.clone(),
            blob_collection: config.blob_keyspace.clone(),
        })
    }

    // -----------------------------------------------------------------------
    // Shared interface — dispatches to Mock or Real variant
    // -----------------------------------------------------------------------

    pub(super) async fn connect(&self) -> anyhow::Result<()> {
        match self {
            Self::Mock(state) => {
                tracing::info!(
                    "Connecting to AmateRS cluster (mock) at {:?}",
                    state.config.cluster_endpoints
                );
                if state.circuit_breaker.is_open().await {
                    state.circuit_breaker.attempt_reset().await;
                    if state.circuit_breaker.is_open().await {
                        return Err(anyhow::anyhow!("Circuit breaker is open"));
                    }
                }
                Ok(())
            }
            #[cfg(feature = "amaters-backend")]
            Self::Real { .. } => {
                // The SDK already verified the connection in `new_real`.
                Ok(())
            }
        }
    }

    pub(super) async fn init_keyspaces(&self) -> anyhow::Result<()> {
        match self {
            Self::Mock(state) => {
                tracing::info!(
                    "Initializing keyspaces (mock): {} and {}",
                    state.config.metadata_keyspace,
                    state.config.blob_keyspace
                );
                Ok(())
            }
            #[cfg(feature = "amaters-backend")]
            Self::Real { .. } => {
                // Keyspace management is server-side in AmateRS.
                Ok(())
            }
        }
    }

    pub(super) async fn put(
        &self,
        keyspace: &str,
        key: String,
        value: Vec<u8>,
    ) -> anyhow::Result<()> {
        match self {
            Self::Mock(state) => {
                if state.circuit_breaker.is_open().await {
                    state.circuit_breaker.attempt_reset().await;
                    if state.circuit_breaker.is_open().await {
                        return Err(anyhow::anyhow!(
                            "Circuit breaker is open, rejecting request"
                        ));
                    }
                }

                let store = if keyspace.contains("blob") {
                    &state.blobs
                } else {
                    &state.metadata
                };

                // Retry logic with exponential backoff.
                // (The mock HashMap insert is infallible; the loop structure mirrors
                //  the real retry pattern for future parity.)
                let mut last_error: Option<anyhow::Error> = None;
                for attempt in 0..state.config.max_retries {
                    let insert_result: anyhow::Result<()> = {
                        let mut map = store.write().await;
                        map.insert(key.clone(), value.clone());
                        Ok(())
                    };
                    match insert_result {
                        Ok(()) => {
                            state.circuit_breaker.record_success().await;
                            return Ok(());
                        }
                        Err(e) => {
                            tracing::warn!("Mock put failed (attempt {}): {}", attempt + 1, e);
                            last_error = Some(e);
                            if attempt < state.config.max_retries - 1 {
                                let backoff = 100 * 2_u64.pow(attempt as u32);
                                tokio::time::sleep(tokio::time::Duration::from_millis(backoff))
                                    .await;
                            }
                        }
                    }
                }

                state.circuit_breaker.record_failure().await;
                Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Mock put operation failed")))
            }

            #[cfg(feature = "amaters-backend")]
            Self::Real {
                sdk,
                metadata_collection,
                blob_collection,
            } => {
                let collection = if keyspace.contains("blob") {
                    blob_collection.as_str()
                } else {
                    metadata_collection.as_str()
                };
                let sdk_key = SdkKey::from_str(&key);
                let sdk_value = SdkCipherBlob::new(value);
                sdk.set(collection, &sdk_key, &sdk_value)
                    .await
                    .map_err(|e| anyhow::anyhow!("AmateRS set error: {e}"))
            }
        }
    }

    pub(super) async fn get(&self, keyspace: &str, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        match self {
            Self::Mock(state) => {
                let store = if keyspace.contains("blob") {
                    &state.blobs
                } else {
                    &state.metadata
                };
                let map = store.read().await;
                Ok(map.get(key).cloned())
            }

            #[cfg(feature = "amaters-backend")]
            Self::Real {
                sdk,
                metadata_collection,
                blob_collection,
            } => {
                let collection = if keyspace.contains("blob") {
                    blob_collection.as_str()
                } else {
                    metadata_collection.as_str()
                };
                let sdk_key = SdkKey::from_str(key);
                let result = sdk
                    .get(collection, &sdk_key)
                    .await
                    .map_err(|e| anyhow::anyhow!("AmateRS get error: {e}"))?;
                Ok(result.map(|blob| blob.as_bytes().to_vec()))
            }
        }
    }

    pub(super) async fn delete(&self, keyspace: &str, key: &str) -> anyhow::Result<()> {
        match self {
            Self::Mock(state) => {
                let store = if keyspace.contains("blob") {
                    &state.blobs
                } else {
                    &state.metadata
                };
                let mut map = store.write().await;
                map.remove(key);
                Ok(())
            }

            #[cfg(feature = "amaters-backend")]
            Self::Real {
                sdk,
                metadata_collection,
                blob_collection,
            } => {
                let collection = if keyspace.contains("blob") {
                    blob_collection.as_str()
                } else {
                    metadata_collection.as_str()
                };
                let sdk_key = SdkKey::from_str(key);
                sdk.delete(collection, &sdk_key)
                    .await
                    .map_err(|e| anyhow::anyhow!("AmateRS delete error: {e}"))
            }
        }
    }

    pub(super) async fn list_prefix(
        &self,
        keyspace: &str,
        prefix: &str,
    ) -> anyhow::Result<Vec<String>> {
        match self {
            Self::Mock(state) => {
                let store = if keyspace.contains("blob") {
                    &state.blobs
                } else {
                    &state.metadata
                };
                let map = store.read().await;
                Ok(map
                    .keys()
                    .filter(|k| k.starts_with(prefix))
                    .cloned()
                    .collect())
            }

            #[cfg(feature = "amaters-backend")]
            Self::Real {
                sdk,
                metadata_collection,
                blob_collection,
            } => {
                let collection = if keyspace.contains("blob") {
                    blob_collection.as_str()
                } else {
                    metadata_collection.as_str()
                };

                let start = SdkKey::from_str(prefix);
                // Compute an exclusive upper bound for the prefix range scan.
                // The SDK `range` uses lexicographic ordering; incrementing the last
                // non-0xFF byte gives a tight upper bound.
                let upper_bytes = prefix_upper_bound(prefix).unwrap_or_else(|| vec![0xFF; 32]);
                let end = SdkKey::from_slice(&upper_bytes);

                let pairs = sdk
                    .range(collection, &start, &end)
                    .await
                    .map_err(|e| anyhow::anyhow!("AmateRS range error: {e}"))?;

                // Convert key bytes back to strings; filter defensively.
                let keys = pairs
                    .into_iter()
                    .map(|(k, _v)| k.to_string_lossy())
                    .filter(|s| s.starts_with(prefix))
                    .collect();

                Ok(keys)
            }
        }
    }
}
