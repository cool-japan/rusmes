//! Standard matcher implementations

pub mod composite;
pub mod has_attachment;
pub mod header_contains;
pub mod is_in_blacklist;
pub mod is_in_whitelist;
pub mod recipient_is_local;
pub mod remote_address;
pub mod sender_is;
pub mod size_greater_than;

pub use composite::{AndMatcher, NotMatcher, OrMatcher};
pub use has_attachment::HasAttachmentMatcher;
pub use header_contains::HeaderContainsMatcher;
pub use is_in_blacklist::IsInBlacklistMatcher;
pub use is_in_whitelist::IsInWhitelistMatcher;
pub use recipient_is_local::RecipientIsLocalMatcher;
pub use remote_address::RemoteAddressMatcher;
pub use sender_is::SenderIsMatcher;
pub use size_greater_than::SizeGreaterThanMatcher;
