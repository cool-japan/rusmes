//! Standard mailet implementations

pub mod add_header;
pub mod bounce;
pub mod dkim_verify;
pub mod dmarc_verify;
pub mod dnsbl;
pub mod forward;
pub mod greylist;
pub mod legalis;
pub mod local_delivery;
pub mod oxify;
pub mod remote_delivery;
pub mod remove_mime_header;
pub mod sieve;
pub mod spam_assassin;
pub mod spf_check;
pub mod virus_scan;

pub use add_header::AddHeaderMailet;
pub use bounce::BounceMailet;
pub use dkim_verify::DkimVerifyMailet;
pub use dmarc_verify::DmarcVerifyMailet;
pub use dnsbl::DnsblMailet;
pub use forward::ForwardMailet;
pub use greylist::GreylistMailet;
pub use legalis::LegalisMailet;
pub use local_delivery::LocalDeliveryMailet;
pub use oxify::OxiFYMailet;
pub use remote_delivery::RemoteDeliveryMailet;
pub use remove_mime_header::RemoveMimeHeaderMailet;
pub use sieve::SieveMailet;
pub use spam_assassin::SpamAssassinMailet;
pub use spf_check::SpfCheckMailet;
pub use virus_scan::VirusScanMailet;
