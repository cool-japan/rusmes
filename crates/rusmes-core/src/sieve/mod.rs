//! Sieve mail filtering language support (RFC 5228)

pub mod interpreter;
pub mod parser;

pub use interpreter::{SieveAction, SieveContext, SieveInterpreter};
pub use parser::{SieveCommand, SieveScript, SieveTest, SieveValue};
