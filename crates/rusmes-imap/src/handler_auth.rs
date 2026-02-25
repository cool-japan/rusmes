//! IMAP authentication command handlers (LOGIN, LOGOUT)

use crate::handler::HandlerContext;
use crate::response::ImapResponse;
use crate::session::{ImapSession, ImapState};
use rusmes_proto::Username;

/// Handle LOGIN command
pub(crate) async fn handle_login(
    ctx: &HandlerContext,
    session: &mut ImapSession,
    tag: &str,
    user: &str,
    password: &str,
) -> anyhow::Result<ImapResponse> {
    // Must be in NotAuthenticated state
    if !matches!(session.state(), ImapState::NotAuthenticated) {
        return Ok(ImapResponse::bad(tag, "Already authenticated"));
    }

    // Authenticate with auth backend
    let username = match Username::new(user.to_string()) {
        Ok(u) => u,
        Err(_) => {
            return Ok(ImapResponse::no(
                tag,
                "[AUTHENTICATIONFAILED] Invalid username",
            ));
        }
    };

    let authenticated = ctx.auth_backend.authenticate(&username, password).await?;

    if authenticated {
        session.state = ImapState::Authenticated;
        session.tag = Some(tag.to_string());
        session.username = Some(username);
        Ok(ImapResponse::ok(tag, format!("{} logged in", user)))
    } else {
        Ok(ImapResponse::no(
            tag,
            "[AUTHENTICATIONFAILED] Invalid credentials",
        ))
    }
}

/// Handle LOGOUT command
pub(crate) async fn handle_logout(
    session: &mut ImapSession,
    tag: &str,
) -> anyhow::Result<ImapResponse> {
    session.state = ImapState::Logout;
    Ok(ImapResponse::ok(tag, "Logout completed"))
}
