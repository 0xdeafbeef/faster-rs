use crate::{FasterError, FasterKv};
use std::ffi::CString;
use std::time::Duration;

// FASTER C++ uses a fixed 2^25 log page size; ensure sizes align to avoid undefined behavior.
const LOG_PAGE_SIZE_BYTES: u64 = 1 << 25;

/// Read-cache configuration.
///
/// See [Tuning FasterKV: Configuring the Read Cache](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-read-cache).
#[derive(bon::Builder, Debug)]
pub struct ReadCacheConfig {
    /// Total read-cache memory size in bytes.
    ///
    /// See [Configuring the Read Cache](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-read-cache).
    pub mem_size: u64,
    /// Fraction of the read cache treated as "second chance" space (0, 1].
    ///
    /// See [Configuring the Read Cache](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-read-cache).
    pub mutable_fraction: f64,
    /// Whether to pre-allocate the read-cache log.
    ///
    /// See [Configuring the Read Cache](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-read-cache).
    #[builder(default)]
    pub pre_allocate: bool,
}

/// Hybrid-log compaction configuration.
///
/// See [Tuning FasterKV: Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
#[derive(bon::Builder, Debug)]
pub struct HlogCompactionConfig {
    /// How often to check whether compaction should run.
    ///
    /// See [Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
    pub check_interval: Duration,
    /// Trigger compaction when the log reaches this fraction of the size budget.
    ///
    /// See [Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
    pub trigger_pct: f64,
    /// Fraction of the log to compact once compaction is triggered.
    ///
    /// See [Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
    pub compact_pct: f64,
    /// Maximum number of bytes to compact in a single run.
    ///
    /// See [Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
    pub max_compacted_size: u64,
    /// Total log size budget in bytes for triggering compaction.
    ///
    /// See [Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
    pub hlog_size_budget: u64,
    /// Number of threads to use for compaction.
    ///
    /// See [Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
    pub num_threads: u8,
}

/// Full FasterKV configuration for the Rust builder.
///
/// See [Tuning FasterKV](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/).
#[derive(bon::Builder, Debug)]
#[builder(finish_fn(vis = "", name = build_internal))]
pub struct FasterKvConfig {
    /// Hash index size in number of buckets.
    ///
    /// See [Managing Hash Index Size](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#managing-hash-index-size).
    pub table_size: u64,
    /// Hybrid-log in-memory size in bytes.
    ///
    /// See [Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
    pub log_size: u64,
    /// Storage directory for the hybrid log; `None` keeps the store in memory.
    ///
    /// See [Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
    pub storage_path: Option<String>,
    /// Fraction of the log treated as mutable in-memory space (0, 1].
    ///
    /// See [Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
    #[builder(default = 0.9)]
    pub log_mutable_fraction: f64,
    /// Whether to pre-allocate the hybrid log on disk.
    ///
    /// See [Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
    #[builder(default)]
    pub pre_allocate_log: bool,
    /// Optional read-cache configuration.
    ///
    /// See [Configuring the Read Cache](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-read-cache).
    pub read_cache: Option<ReadCacheConfig>,
    /// Optional hybrid-log compaction configuration.
    ///
    /// See [Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
    pub hlog_compaction: Option<HlogCompactionConfig>,
}

impl FasterKvConfig {
    fn validate(&self) -> Result<(), FasterError<'static>> {
        if !self.table_size.is_power_of_two() {
            return Err(FasterError::BuilderError("Index table size must be a power of two"));
        }
        if self.table_size > i32::MAX as u64 {
            return Err(FasterError::BuilderError("Index table size is too large"));
        }
        if self.log_size < LOG_PAGE_SIZE_BYTES {
            return Err(FasterError::BuilderError(
                "Log size must be at least one log page",
            ));
        }
        if !self.log_size.is_multiple_of(LOG_PAGE_SIZE_BYTES) {
            return Err(FasterError::BuilderError(
                "Log size must be a multiple of the log page size",
            ));
        }
        if self.log_size / LOG_PAGE_SIZE_BYTES > u32::MAX as u64 {
            return Err(FasterError::BuilderError(
                "Log size is too large for the log page index",
            ));
        }
        if !(self.log_mutable_fraction > 0.0 && self.log_mutable_fraction <= 1.0) {
            return Err(FasterError::BuilderError(
                "Log mutable fraction must be between 0 and 1",
            ));
        }
        if let Some(path) = &self.storage_path {
            if path.is_empty() {
                return Err(FasterError::BuilderError("Storage path must not be empty"));
            }
            if CString::new(path.as_str()).is_err() {
                return Err(FasterError::BuilderError(
                    "Storage path contains an interior NUL byte",
                ));
            }
        }
        if let Some(read_cache) = &self.read_cache {
            if read_cache.mem_size < LOG_PAGE_SIZE_BYTES {
                return Err(FasterError::BuilderError(
                    "Read cache size must be at least one log page",
                ));
            }
            if !read_cache.mem_size.is_multiple_of(LOG_PAGE_SIZE_BYTES) {
                return Err(FasterError::BuilderError(
                    "Read cache size must be a multiple of the log page size",
                ));
            }
            if read_cache.mem_size / LOG_PAGE_SIZE_BYTES > u32::MAX as u64 {
                return Err(FasterError::BuilderError(
                    "Read cache size is too large for the log page index",
                ));
            }
            if !(read_cache.mutable_fraction > 0.0 && read_cache.mutable_fraction <= 1.0) {
                return Err(FasterError::BuilderError(
                    "Read cache mutable fraction must be between 0 and 1",
                ));
            }
        }
        if let Some(hlog_compaction) = &self.hlog_compaction {
            if hlog_compaction.check_interval == Duration::ZERO {
                return Err(FasterError::BuilderError(
                    "Compaction check interval must be greater than zero",
                ));
            }
            if hlog_compaction.check_interval.as_millis() > u64::MAX as u128 {
                return Err(FasterError::BuilderError(
                    "Compaction check interval is too large",
                ));
            }
            if !(hlog_compaction.trigger_pct > 0.0 && hlog_compaction.trigger_pct <= 1.0) {
                return Err(FasterError::BuilderError(
                    "Compaction trigger percent must be between 0 and 1",
                ));
            }
            if !(hlog_compaction.compact_pct > 0.0 && hlog_compaction.compact_pct <= 1.0) {
                return Err(FasterError::BuilderError(
                    "Compaction percent must be between 0 and 1",
                ));
            }
            if hlog_compaction.max_compacted_size == 0 {
                return Err(FasterError::BuilderError(
                    "Compaction max compacted size must be greater than zero",
                ));
            }
            if hlog_compaction.hlog_size_budget == 0 {
                return Err(FasterError::BuilderError(
                    "Compaction log size budget must be greater than zero",
                ));
            }
            if hlog_compaction.num_threads == 0 {
                return Err(FasterError::BuilderError(
                    "Compaction thread count must be greater than zero",
                ));
            }
        }
        Ok(())
    }

    fn open(&self) -> Result<FasterKv, FasterError<'static>> {
        self.validate()?;

        let storage = match &self.storage_path {
            Some(path) => Some(
                CString::new(path.as_str())
                    .map_err(|_| FasterError::BuilderError("Storage path contains an interior NUL byte"))?,
            ),
            None => None,
        };
        let storage_ptr = storage
            .as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr());

        let read_cache = match &self.read_cache {
            Some(read_cache) => ffi::faster_read_cache_config {
                enabled: true,
                mem_size: read_cache.mem_size,
                mutable_fraction: read_cache.mutable_fraction,
                pre_allocate: read_cache.pre_allocate,
            },
            None => ffi::faster_read_cache_config {
                enabled: false,
                mem_size: 0,
                mutable_fraction: 0.0,
                pre_allocate: false,
            },
        };

        let hlog_compaction = match &self.hlog_compaction {
            Some(hlog_compaction) => ffi::faster_hlog_compaction_config {
                enabled: true,
                check_interval_ms: hlog_compaction.check_interval.as_millis() as u64,
                trigger_pct: hlog_compaction.trigger_pct,
                compact_pct: hlog_compaction.compact_pct,
                max_compacted_size: hlog_compaction.max_compacted_size,
                hlog_size_budget: hlog_compaction.hlog_size_budget,
                num_threads: hlog_compaction.num_threads,
            },
            None => ffi::faster_hlog_compaction_config {
                enabled: false,
                check_interval_ms: 0,
                trigger_pct: 0.0,
                compact_pct: 0.0,
                max_compacted_size: 0,
                hlog_size_budget: 0,
                num_threads: 0,
            },
        };

        let config = ffi::faster_kv_config {
            table_size: self.table_size,
            log_size: self.log_size,
            storage: storage_ptr,
            log_mutable_fraction: self.log_mutable_fraction,
            pre_allocate_log: self.pre_allocate_log,
            read_cache,
            hlog_compaction,
        };

        let faster_t = unsafe { ffi::faster_open_with_config(&config) };
        if faster_t.is_null() {
            return Err(FasterError::BuilderError(
                "faster_open_with_config returned null",
            ));
        }

        Ok(FasterKv {
            faster_t,
            storage_dir: self.storage_path.clone(),
        })
    }
}

impl<S: faster_kv_config_builder::IsComplete> FasterKvConfigBuilder<S> {
    pub fn build(self) -> Result<FasterKv, FasterError<'static>> {
        let config = self.build_internal();
        config.open()
    }
}

#[cfg(test)]
pub mod tests {
    use super::{FasterKvConfig, HlogCompactionConfig, ReadCacheConfig, LOG_PAGE_SIZE_BYTES};
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn can_build_with_disk() {
        let dir = TempDir::new().unwrap();
        let dir_str = dir.path().to_str().unwrap();
        let kv = FasterKvConfig::builder()
            .table_size(1 << 15)
            .log_size(1024 * 1024 * 1024)
            .storage_path(dir_str.to_owned())
            .pre_allocate_log(true)
            .log_mutable_fraction(0.8)
            .build()
            .unwrap();
        let storage = &kv.storage_dir;
        assert_eq!(storage.as_ref().unwrap(), dir_str);
    }

    #[test]
    fn rejects_non_power_of_two_table_size() {
        let err = FasterKvConfig::builder()
            .table_size(3)
            .log_size(LOG_PAGE_SIZE_BYTES)
            .build()
            .err()
            .unwrap();
        assert_eq!(format!("{err}"), "Builder error: Index table size must be a power of two");
    }

    #[test]
    fn rejects_unaligned_log_size() {
        let err = FasterKvConfig::builder()
            .table_size(1 << 10)
            .log_size(LOG_PAGE_SIZE_BYTES + 1)
            .build()
            .err()
            .unwrap();
        assert_eq!(
            format!("{err}"),
            "Builder error: Log size must be a multiple of the log page size"
        );
    }

    #[test]
    fn rejects_read_cache_without_size() {
        let err = FasterKvConfig::builder()
            .table_size(1 << 10)
            .log_size(LOG_PAGE_SIZE_BYTES)
            .read_cache(
                ReadCacheConfig::builder()
                    .mem_size(0)
                    .mutable_fraction(0.5)
                    .build(),
            )
            .build()
            .err()
            .unwrap();
        assert_eq!(
            format!("{err}"),
            "Builder error: Read cache size must be at least one log page"
        );
    }

    #[test]
    fn rejects_read_cache_bad_fraction() {
        let err = FasterKvConfig::builder()
            .table_size(1 << 10)
            .log_size(LOG_PAGE_SIZE_BYTES)
            .read_cache(
                ReadCacheConfig::builder()
                    .mem_size(LOG_PAGE_SIZE_BYTES)
                    .mutable_fraction(0.0)
                    .build(),
            )
            .build()
            .err()
            .unwrap();
        assert_eq!(
            format!("{err}"),
            "Builder error: Read cache mutable fraction must be between 0 and 1"
        );
    }

    #[test]
    fn rejects_compaction_without_threads() {
        let err = FasterKvConfig::builder()
            .table_size(1 << 10)
            .log_size(LOG_PAGE_SIZE_BYTES)
            .hlog_compaction(
                HlogCompactionConfig::builder()
                    .check_interval(Duration::from_millis(1))
                    .trigger_pct(0.5)
                    .compact_pct(0.2)
                    .max_compacted_size(LOG_PAGE_SIZE_BYTES)
                    .hlog_size_budget(LOG_PAGE_SIZE_BYTES * 2)
                    .num_threads(0)
                    .build(),
            )
            .build()
            .err()
            .unwrap();
        assert_eq!(
            format!("{err}"),
            "Builder error: Compaction thread count must be greater than zero"
        );
    }

    #[test]
    fn rejects_compaction_trigger_percent() {
        let err = FasterKvConfig::builder()
            .table_size(1 << 10)
            .log_size(LOG_PAGE_SIZE_BYTES)
            .hlog_compaction(
                HlogCompactionConfig::builder()
                    .check_interval(Duration::from_millis(1))
                    .trigger_pct(1.5)
                    .compact_pct(0.2)
                    .max_compacted_size(LOG_PAGE_SIZE_BYTES)
                    .hlog_size_budget(LOG_PAGE_SIZE_BYTES * 2)
                    .num_threads(1)
                    .build(),
            )
            .build()
            .err()
            .unwrap();
        assert_eq!(
            format!("{err}"),
            "Builder error: Compaction trigger percent must be between 0 and 1"
        );
    }

    #[test]
    fn rejects_empty_storage_path() {
        let err = FasterKvConfig::builder()
            .table_size(1 << 10)
            .log_size(LOG_PAGE_SIZE_BYTES)
            .storage_path("".to_owned())
            .build()
            .err()
            .unwrap();
        assert_eq!(
            format!("{err}"),
            "Builder error: Storage path must not be empty"
        );
    }

    #[test]
    fn rejects_log_size_too_small() {
        let err = FasterKvConfig::builder()
            .table_size(1 << 10)
            .log_size(LOG_PAGE_SIZE_BYTES - 1)
            .build()
            .err()
            .unwrap();
        assert_eq!(
            format!("{err}"),
            "Builder error: Log size must be at least one log page"
        );
    }

    #[test]
    fn rejects_log_mutable_fraction_zero() {
        let err = FasterKvConfig::builder()
            .table_size(1 << 10)
            .log_size(LOG_PAGE_SIZE_BYTES)
            .log_mutable_fraction(0.0)
            .build()
            .err()
            .unwrap();
        assert_eq!(
            format!("{err}"),
            "Builder error: Log mutable fraction must be between 0 and 1"
        );
    }

    #[test]
    fn rejects_log_size_too_large() {
        let max_pages: u64 = u32::MAX as u64;
        let err = FasterKvConfig::builder()
            .table_size(1 << 10)
            .log_size(LOG_PAGE_SIZE_BYTES * (max_pages + 1))
            .build()
            .err()
            .unwrap();
        assert_eq!(
            format!("{err}"),
            "Builder error: Log size is too large for the log page index"
        );
    }
}
