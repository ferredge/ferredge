//! Storage backend abstraction.
//!
//! This module defines the interface for persisting readings and events
//! to various storage backends (TimescaleDB, InfluxDB, SQLite, Parquet, etc.).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::device::DeviceId;
use crate::error::Result;
use crate::message::Reading;

/// Query parameters for retrieving readings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadingQuery {
    /// Filter by device ID
    pub device_id: Option<DeviceId>,
    /// Filter by resource name
    pub resource: Option<String>,
    /// Start time (inclusive)
    pub start: Option<DateTime<Utc>>,
    /// End time (exclusive)
    pub end: Option<DateTime<Utc>>,
    /// Maximum number of results
    pub limit: Option<usize>,
    /// Offset for pagination
    pub offset: Option<usize>,
    /// Order by time ascending (default: descending)
    pub ascending: bool,
}

impl ReadingQuery {
    /// Creates a new query builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Filters by device ID.
    pub fn device(mut self, device_id: DeviceId) -> Self {
        self.device_id = Some(device_id);
        self
    }

    /// Filters by resource name.
    pub fn resource(mut self, resource: impl Into<String>) -> Self {
        self.resource = Some(resource.into());
        self
    }

    /// Sets the time range.
    pub fn time_range(mut self, start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        self.start = Some(start);
        self.end = Some(end);
        self
    }

    /// Sets the limit.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Sets the offset.
    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Orders by time ascending.
    pub fn ascending(mut self) -> Self {
        self.ascending = true;
        self
    }
}

/// Result of a query operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// The readings returned
    pub readings: Vec<Reading>,
    /// Total count (for pagination)
    pub total: Option<usize>,
    /// Whether there are more results
    pub has_more: bool,
}

/// Aggregation functions for time-series data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Aggregation {
    /// Average value
    Avg,
    /// Sum of values
    Sum,
    /// Minimum value
    Min,
    /// Maximum value
    Max,
    /// Count of readings
    Count,
    /// First value in window
    First,
    /// Last value in window
    Last,
}

/// Parameters for aggregated queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateQuery {
    /// Base query parameters
    pub base: ReadingQuery,
    /// Aggregation function
    pub aggregation: Aggregation,
    /// Time bucket size in seconds
    pub bucket_seconds: u64,
}

/// The storage backend trait.
///
/// Implementations provide persistence for readings and support
/// for time-series queries.
pub trait StorageBackend: Send + Sync {
    /// Returns the backend name.
    fn name(&self) -> &str;

    /// Initializes the storage backend.
    fn init(&self) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Closes the storage backend.
    fn close(&self) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Stores a single reading.
    fn store(&self, reading: Reading) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Stores multiple readings in a batch.
    fn store_batch(&self, readings: Vec<Reading>) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Queries readings.
    fn query(&self, query: ReadingQuery) -> impl std::future::Future<Output = Result<QueryResult>> + Send;

    /// Gets the latest reading for a device resource.
    fn latest(
        &self,
        device_id: &DeviceId,
        resource: &str,
    ) -> impl std::future::Future<Output = Result<Option<Reading>>> + Send;

    /// Deletes readings matching the query.
    fn delete(&self, query: ReadingQuery) -> impl std::future::Future<Output = Result<usize>> + Send;

    /// Performs an aggregated query (optional, may not be supported by all backends).
    fn aggregate(
        &self,
        _query: AggregateQuery,
    ) -> impl std::future::Future<Output = Result<Vec<Reading>>> + Send {
        async {
            Err(crate::error::StorageError::NotAvailable(
                "Aggregation not supported by this backend".to_string(),
            )
            .into())
        }
    }
}

/// A type-erased storage backend reference.
pub type DynStorageBackend = std::sync::Arc<dyn StorageBackend>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reading_query_builder() {
        let query = ReadingQuery::new()
            .device(DeviceId::new("device-1"))
            .resource("temperature")
            .limit(100)
            .ascending();

        assert_eq!(query.device_id, Some(DeviceId::new("device-1")));
        assert_eq!(query.resource, Some("temperature".to_string()));
        assert_eq!(query.limit, Some(100));
        assert!(query.ascending);
    }
}
