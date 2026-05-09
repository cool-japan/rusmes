//! Tests for per-protocol TLS configuration (Item 2).

use rusmes_config::{ClientAuthMode, ProtocolKind, ServerConfig, TlsConfig, TlsEndpointConfig};
use std::path::PathBuf;

fn minimal_toml_with(extra: &str) -> String {
    format!(
        r#"
domain = "example.com"
postmaster = "postmaster@example.com"

[smtp]
host = "0.0.0.0"
port = 25
max_message_size = "50MB"

[storage]
backend = "filesystem"
path = "/var/mail"

[[processors]]
name = "root"
state = "root"

[[processors.mailets]]
matcher = "All"
mailet = "LocalDelivery"

{extra}
"#
    )
}

#[test]
fn test_per_protocol_tls_fallback() {
    // Config with only [tls.default] should resolve to the same cert for all protocols.
    let extra = r#"
[tls.default]
cert_path = "/etc/rusmes/tls/default.crt"
key_path  = "/etc/rusmes/tls/default.key"
"#;
    let config: ServerConfig = toml::from_str(&minimal_toml_with(extra)).unwrap();
    let tls = config.tls.as_ref().expect("tls section should be present");

    let smtp_ep = tls.tls_for_protocol(ProtocolKind::Smtp);
    let imap_ep = tls.tls_for_protocol(ProtocolKind::Imap);
    let pop3_ep = tls.tls_for_protocol(ProtocolKind::Pop3);
    let jmap_ep = tls.tls_for_protocol(ProtocolKind::Jmap);

    assert_eq!(
        smtp_ep.cert_path,
        PathBuf::from("/etc/rusmes/tls/default.crt")
    );
    assert_eq!(
        imap_ep.cert_path,
        PathBuf::from("/etc/rusmes/tls/default.crt")
    );
    assert_eq!(
        pop3_ep.cert_path,
        PathBuf::from("/etc/rusmes/tls/default.crt")
    );
    assert_eq!(
        jmap_ep.cert_path,
        PathBuf::from("/etc/rusmes/tls/default.crt")
    );

    // Also verify via the ServerConfig helper
    let via_helper = config
        .tls_for_protocol(ProtocolKind::Imap)
        .expect("tls_for_protocol should return Some");
    assert_eq!(
        via_helper.cert_path,
        PathBuf::from("/etc/rusmes/tls/default.crt")
    );
}

#[test]
fn test_per_protocol_tls_override_imap() {
    // A [tls.imap] section should be returned for IMAP; other protocols fall back to default.
    let extra = r#"
[tls.default]
cert_path = "/etc/rusmes/tls/default.crt"
key_path  = "/etc/rusmes/tls/default.key"

[tls.imap]
cert_path = "/etc/rusmes/tls/imap.crt"
key_path  = "/etc/rusmes/tls/imap.key"
"#;
    let config: ServerConfig = toml::from_str(&minimal_toml_with(extra)).unwrap();
    let tls = config.tls.as_ref().expect("tls section should be present");

    let imap_ep = tls.tls_for_protocol(ProtocolKind::Imap);
    let smtp_ep = tls.tls_for_protocol(ProtocolKind::Smtp);

    assert_eq!(imap_ep.cert_path, PathBuf::from("/etc/rusmes/tls/imap.crt"));
    assert_eq!(
        smtp_ep.cert_path,
        PathBuf::from("/etc/rusmes/tls/default.crt")
    );
}

#[test]
fn test_per_protocol_tls_override_all() {
    // All four protocols can each have independent certs.
    let extra = r#"
[tls.default]
cert_path = "/etc/tls/default.crt"
key_path  = "/etc/tls/default.key"

[tls.smtp]
cert_path = "/etc/tls/smtp.crt"
key_path  = "/etc/tls/smtp.key"

[tls.imap]
cert_path = "/etc/tls/imap.crt"
key_path  = "/etc/tls/imap.key"

[tls.pop3]
cert_path = "/etc/tls/pop3.crt"
key_path  = "/etc/tls/pop3.key"

[tls.jmap]
cert_path = "/etc/tls/jmap.crt"
key_path  = "/etc/tls/jmap.key"
"#;
    let config: ServerConfig = toml::from_str(&minimal_toml_with(extra)).unwrap();
    let tls = config.tls.as_ref().expect("tls section should be present");

    assert_eq!(
        tls.tls_for_protocol(ProtocolKind::Smtp).cert_path,
        PathBuf::from("/etc/tls/smtp.crt")
    );
    assert_eq!(
        tls.tls_for_protocol(ProtocolKind::Imap).cert_path,
        PathBuf::from("/etc/tls/imap.crt")
    );
    assert_eq!(
        tls.tls_for_protocol(ProtocolKind::Pop3).cert_path,
        PathBuf::from("/etc/tls/pop3.crt")
    );
    assert_eq!(
        tls.tls_for_protocol(ProtocolKind::Jmap).cert_path,
        PathBuf::from("/etc/tls/jmap.crt")
    );
}

#[test]
fn test_no_tls_section_is_none() {
    // Configs without a [tls] section should yield None, not an error.
    let config: ServerConfig = toml::from_str(&minimal_toml_with("")).unwrap();
    assert!(config.tls.is_none());
    assert!(config.tls_for_protocol(ProtocolKind::Smtp).is_none());
}

#[test]
fn test_tls_endpoint_validate_ok() {
    let ep = TlsEndpointConfig {
        cert_path: PathBuf::from("/etc/tls/cert.pem"),
        key_path: PathBuf::from("/etc/tls/key.pem"),
        client_auth: ClientAuthMode::Disabled,
        client_ca_path: None,
    };
    ep.validate().unwrap();
}

#[test]
fn test_tls_endpoint_validate_empty_cert() {
    let ep = TlsEndpointConfig {
        cert_path: PathBuf::from(""),
        key_path: PathBuf::from("/etc/tls/key.pem"),
        client_auth: ClientAuthMode::Disabled,
        client_ca_path: None,
    };
    assert!(ep.validate().is_err());
}

#[test]
fn test_tls_config_validate_ok() {
    let cfg = TlsConfig {
        default: TlsEndpointConfig {
            cert_path: PathBuf::from("/etc/tls/cert.pem"),
            key_path: PathBuf::from("/etc/tls/key.pem"),
            client_auth: ClientAuthMode::Disabled,
            client_ca_path: None,
        },
        smtp: None,
        imap: None,
        pop3: None,
        jmap: None,
    };
    cfg.validate().unwrap();
}

#[test]
fn test_tls_endpoint_validate_mtls_required_needs_ca() {
    use rusmes_config::ClientAuthMode;
    let ep = TlsEndpointConfig {
        cert_path: PathBuf::from("/etc/tls/cert.pem"),
        key_path: PathBuf::from("/etc/tls/key.pem"),
        client_auth: ClientAuthMode::Required,
        client_ca_path: None, // missing CA path
    };
    assert!(
        ep.validate().is_err(),
        "Required mode without ca_path must fail validation"
    );
}

#[test]
fn test_tls_endpoint_validate_mtls_required_with_ca() {
    use rusmes_config::ClientAuthMode;
    let ep = TlsEndpointConfig {
        cert_path: PathBuf::from("/etc/tls/cert.pem"),
        key_path: PathBuf::from("/etc/tls/key.pem"),
        client_auth: ClientAuthMode::Required,
        client_ca_path: Some(PathBuf::from("/etc/tls/ca.pem")),
    };
    assert!(
        ep.validate().is_ok(),
        "Required mode with ca_path must pass validation"
    );
}
