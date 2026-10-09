use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{errors::AppError, middleware::auth::CurrentUser};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Required {
    Open,
    Admin,
    Ferris,
}

/// Admin-level areas (spec section 8). EVERYTHING else under /admin is Ferris-only,
/// so a new admin route that someone forgets to list fails closed.
const ADMIN_LEVEL: &[&str] = &[
    "/admin/dashboard",
    "/admin/teams",
    "/admin/users",
    "/admin/export",
    "/admin/audit",
];

pub fn required_level(path: &str) -> Required {
    // Normalize defensively: case and repeated leading slashes never lower the bar.
    let p = format!("/{}", path.trim_start_matches('/').to_ascii_lowercase());
    if p != "/admin" && !p.starts_with("/admin/") {
        return Required::Open;
    }
    if p == "/admin/login" {
        return Required::Open; // the one public admin route (spec 9.3)
    }
    let admin_level = ADMIN_LEVEL
        .iter()
        .any(|&base| p == base || p.strip_prefix(base).is_some_and(|rest| rest.starts_with('/')));
    if admin_level {
        Required::Admin
    } else {
        Required::Ferris
    }
}

/// Path-based gate, independent of routing. Non-privileged callers get exactly the
/// response a missing route gets, whether or not the admin route exists.
pub async fn admin_gate(req: Request, next: Next) -> Response {
    let need = required_level(req.uri().path());
    if need != Required::Open {
        let allowed = match req.extensions().get::<CurrentUser>().and_then(|c| c.0.as_ref()) {
            Some(u) => match need {
                Required::Admin => u.is_admin,
                Required::Ferris => u.is_ferris,
                Required::Open => true,
            },
            None => false,
        };
        if !allowed {
            tracing::warn!(path = %req.uri().path(), "blocked non-privileged access to admin route");
            return AppError::NotFound("admin route".into()).into_response();
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        for (p, want) in [
            ("/", Required::Open),
            ("/login", Required::Open),
            ("/challenges/1", Required::Open),
            ("/administrator", Required::Open),
            ("/admin/login", Required::Open),
            ("/admin/dashboard", Required::Admin),
            ("/admin/teams", Required::Admin),
            ("/admin/teams/x/score", Required::Admin),
            ("/admin/users/x/action", Required::Admin),
            ("/admin/export/solves", Required::Admin),
            ("/admin/audit", Required::Admin),
            ("/admin/rounds", Required::Ferris),
            ("/admin/rounds/start", Required::Ferris),
            ("/admin/challenges/new", Required::Ferris),
            ("/admin/hints/x/edit", Required::Ferris),
            ("/admin/event/start", Required::Ferris),
            ("/admin/admins/add", Required::Ferris),
            ("/admin", Required::Ferris),
            ("/admin/", Required::Ferris),
            ("/admin/unknown-new-route", Required::Ferris),
            ("/admin/dashboardx", Required::Ferris),
            ("/admin/login/", Required::Ferris),
            ("/admin/login/x", Required::Ferris),
        ] {
            assert_eq!(required_level(p), want, "{p}");
        }
    }

    #[test]
    fn evasion_attempts_are_still_gated() {
        assert_eq!(required_level("//admin/dashboard"), Required::Admin);
        assert_eq!(required_level("/ADMIN/Rounds"), Required::Ferris);
    }
}
