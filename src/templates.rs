use axum::{
    body::Body,
    extract::Path,
    http::{header, HeaderValue},
    response::{Html, IntoResponse, Response},
};
use minijinja::Environment;
use rust_embed::RustEmbed;
use serde::Serialize;

use crate::{
    errors::{AppError, AppResult},
    session::SessionUser,
};

// Debug builds read these folders from disk; release builds embed them in the binary.
#[derive(RustEmbed)]
#[folder = "templates/"]
struct TemplateFiles;

#[derive(RustEmbed)]
#[folder = "static/"]
struct StaticFiles;

/// Every template the app renders. Checked at startup so a missing or syntactically
/// broken template stops the boot instead of failing on a live request.
const REQUIRED: &[&str] = &[
    "base.html",
    "index.html",
    "auth/login.html",
    "auth/register.html",
    "admin/login.html",
    "admin/dashboard.html",
    "admin/teams.html",
    "admin/rounds.html",
    "admin/challenges.html",
    "admin/admins.html",
    "admin/audit.html",
    "onboarding/index.html",
    "onboarding/preview.html",
    "team/dashboard.html",
    "challenges/list.html",
    "challenges/detail.html",
    "scoreboard.html",
    "profile.html",
    "errors/error.html",
];

/// What templates may know about the logged-in user. Nothing sensitive.
#[derive(Serialize)]
pub struct UserView {
    pub display_name: String,
    pub username: String,
    pub is_admin: bool,
    pub is_ferris: bool,
}

impl From<&SessionUser> for UserView {
    fn from(u: &SessionUser) -> Self {
        Self {
            display_name: u.display_name.clone(),
            username: u.username.clone(),
            is_admin: u.is_admin,
            is_ferris: u.is_ferris,
        }
    }
}

pub struct Templates {
    env: Environment<'static>,
}

impl Templates {
    pub fn new() -> Self {
        let mut env = Environment::new();
        // Auto-escaping is on for *.html by default.
        env.set_loader(|name| {
            Ok(TemplateFiles::get(name).map(|f| String::from_utf8_lossy(&f.data).into_owned()))
        });
        Self { env }
    }

    pub fn check(&self) -> Result<(), minijinja::Error> {
        for name in REQUIRED {
            self.env.get_template(name)?;
        }
        Ok(())
    }

    pub fn render<S: Serialize>(&self, name: &str, ctx: S) -> AppResult<Html<String>> {
        let tmpl = self.env.get_template(name)?;
        Ok(Html(tmpl.render(ctx)?))
    }
}

/// GET /static/*path, served from the embedded folder.
pub async fn static_file(Path(path): Path<String>) -> Response {
    let suspicious = path.is_empty()
        || path.starts_with('/')
        || path.contains("..")
        || path.contains('\\')
        || path.contains('\0');
    if suspicious {
        return AppError::NotFound("static: bad path".into()).into_response();
    }
    let Some(file) = StaticFiles::get(&path) else {
        return AppError::NotFound("static: missing".into()).into_response();
    };

    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    let ctype = if mime.type_() == mime_guess::mime::TEXT {
        format!("{mime}; charset=utf-8")
    } else {
        mime.to_string()
    };

    let mut resp = Response::new(Body::from(file.data.into_owned()));
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&ctype) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    resp
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_required_templates_compile() {
        super::Templates::new().check().unwrap();
    }
}
