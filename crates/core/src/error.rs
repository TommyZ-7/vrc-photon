use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CoreError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid data: {0}")]
    Invalid(String),
    /// A row the caller named no longer exists.
    #[error("not found: {entity} {id}")]
    NotFound { entity: &'static str, id: String },
}

pub type Result<T> = std::result::Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_reports_its_entity_and_id() {
        let error = CoreError::NotFound {
            entity: "compression profile",
            id: "work".to_string(),
        };

        assert_eq!(error.to_string(), "not found: compression profile work");
    }

    #[test]
    fn not_found_is_distinguishable_from_invalid_data() {
        let missing = CoreError::NotFound {
            entity: "compression profile",
            id: "work".to_string(),
        };
        let invalid = CoreError::Invalid("bad data".to_string());

        assert!(matches!(missing, CoreError::NotFound { .. }));
        assert!(!matches!(invalid, CoreError::NotFound { .. }));
    }
}
