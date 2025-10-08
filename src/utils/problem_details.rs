use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Standard Problem Details error response format (RFC 7807)
#[derive(Debug, Serialize)]
pub struct ProblemDetails {
    /// URI reference that identifies the problem type
    #[serde(rename = "type")]
    pub problem_type: String,
    /// Short, human-readable summary of the problem type
    pub title: String,
    /// HTTP status code
    pub status: u16,
    /// Human-readable explanation specific to this occurrence
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// URI reference that identifies the specific occurrence of the problem
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Additional members to extend the problem details
    #[serde(flatten)]
    pub errors: serde_json::Map<String, serde_json::Value>,
}

impl ProblemDetails {
    pub fn new(problem_type: String, title: String, status: u16) -> Self {
        Self {
            problem_type,
            title,
            status,
            detail: None,
            instance: None,
            errors: serde_json::Map::new(),
        }
    }

    pub fn with_detail(mut self, detail: String) -> Self {
        self.detail = Some(detail.to_string());
        self
    }

    pub fn with_instance(mut self, instance: String) -> Self {
        self.instance = Some(instance.to_string());
        self
    }

    pub fn with_extension(mut self, key: String, value: serde_json::Value) -> Self {
        self.errors.insert(key.to_string(), value);
        self
    }

    // Helper constructors for common error types
    pub fn validation_error(message: String) -> Self {
        Self::new(
            "https://httpstatuses.io/400".to_string(),
            "Bad Request".to_string(),
            400,
        )
        .with_detail(message)
    }

    pub fn not_found(message: String) -> Self {
        Self::new(
            "https://httpstatuses.io/404".to_string(),
            "Not Found".to_string(),
            404,
        )
        .with_detail(message)
    }

    pub fn conflict(message: String) -> Self {
        Self::new(
            "https://httpstatuses.io/409".to_string(),
            "Conflict".to_string(),
            409,
        )
        .with_detail(message)
    }

    pub fn unauthorized(message: String) -> Self {
        Self::new(
            "https://httpstatuses.io/401".to_string(),
            "Unauthorized".to_string(),
            401,
        )
        .with_detail(message)
    }

    pub fn forbidden(message: String) -> Self {
        Self::new(
            "https://httpstatuses.io/403".to_string(),
            "Forbidden".to_string(),
            403,
        )
        .with_detail(message)
    }

    pub fn method_not_allowed(message: String) -> Self {
        Self::new(
            "https://httpstatuses.io/405".to_string(),
            "Method Not Allowed".to_string(),
            405,
        )
        .with_detail(message)
    }

    pub fn internal_server_error(message: String) -> Self {
        Self::new(
            "https://httpstatuses.io/500".to_string(),
            "Internal Server Error".to_string(),
            500,
        )
        .with_detail(message)
    }

    pub fn unprocessable_entity(message: String) -> Self {
        Self::new(
            "https://httpstatuses.io/422".to_string(),
            "Unprocessable Entity".to_string(),
            422,
        )
        .with_detail(message)
    }
}

impl IntoResponse for ProblemDetails {
    fn into_response(self) -> Response {
        let status = axum::http::StatusCode::from_u16(self.status)
            .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);

        let body = serde_json::to_string(&self).unwrap_or_else(|serialization_err| {
            tracing::error!(
                error = %serialization_err,
                "Failed to serialize error response"
            );
            r#"{"type":"https://httpstatuses.io/500","title":"Internal Server Error","status":500,"detail":"Failed to serialize error response"}"#.to_string()
        });

        (
            status,
            [
                ("content-type", "application/problem+json"),
                ("x-request-id", &uuid::Uuid::now_v7().to_string()), // Add request ID for tracing
            ],
            body,
        )
            .into_response()
    }
}
