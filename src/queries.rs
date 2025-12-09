use serde::{Deserialize, Serialize};

/// wrapper for queries with pagination requirements
#[derive(Debug, Clone)]
pub struct QueryResult<T> {
    // array of results
    pub data: Vec<T>,
    // total result matched for pagination
    pub total: i64,
}

impl<T> QueryResult<T> {
    pub fn new(data: Vec<T>, total: i64) -> Self {
        Self { data, total }
    }
}

/// wrapper for queries with pagination requirements
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PaginatedQuery<T> {
    pub query: T,
    /// page number for pagination (zero indexed)
    pub page: u32,
    pub limit: u32,
}
