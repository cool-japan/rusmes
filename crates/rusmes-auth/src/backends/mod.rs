//! Authentication backend implementations

pub mod ldap;
pub mod oauth2;
pub mod sql;

#[cfg(feature = "pam-auth")]
pub mod pam;
