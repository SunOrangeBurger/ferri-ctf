use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use thiserror::Error;

/// Application error. Every variant maps to a status code, and the client only ever
/// sees a generic message. Details go to the logs, never to the response body.
#[derive(Debug, Error)]
pub enum AppError {
    /// Also used for "you may not know this exists": admin routes, out-of-pool
    /// challenges, other teams' data. Never reveal which case it was (spec 9.3).
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized")]
    Unauthorized,

    /// Logged in but not permitted, for player-facing actions only. Never for /admin/*.
    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("payload too large")]
    PayloadTooLarge,

    #[error("rate limited")]
    TooManyRequests,

    /// Argon2 semaphore exhausted (spec 9.5).
    #[error("service busy")]
    ServiceUnavailable,

    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("template error: {0}")]
    Template(#[from] minijinja::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type AppResult<T> = Result<T, AppError>;

/// Attached to every error response so the error-page middleware can swap the plain
/// body for the themed page. Carries only text that is safe to show.
#[derive(Clone, Debug)]
pub struct PublicError {
    pub status: StatusCode,
    pub message: String,
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            Self::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Db(_) | Self::Io(_) | Self::Template(_) | Self::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    /// Safe text for the client. Client errors show what the handler wrote (handlers must
    /// keep these free of secrets); server errors show nothing specific.
    fn public_message(&self) -> String {
        match self {
            Self::NotFound(_) => "Not found".into(),
            Self::BadRequest(m) | Self::Forbidden(m) | Self::Conflict(m) => m.clone(),
            Self::Unauthorized => "Please log in".into(),
            Self::PayloadTooLarge => "File too large".into(),
            Self::TooManyRequests => "Too many requests, slow down".into(),
            Self::ServiceUnavailable => "Server busy, try again shortly".into(),
            Self::Db(_) | Self::Io(_) | Self::Template(_) | Self::Internal(_) => {
                "Something went wrong".into()
            }
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();

        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        } else {
            tracing::debug!(error = %self, "request rejected");
        }

        let message = self.public_message();
        let mut resp = (status, message.clone()).into_response();
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        // The error-page middleware replaces the plain body with the themed page.
        resp.extensions_mut().insert(PublicError { status, message });
        resp
    }
}

impl From<axum::http::header::InvalidHeaderValue> for AppError {
    fn from(e: axum::http::header::InvalidHeaderValue) -> Self {
        Self::Internal(format!("invalid header value: {e}"))
    }
}

impl From<tokio::task::JoinError> for AppError {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::Internal(format!("task join failed: {e}"))
    }
}
