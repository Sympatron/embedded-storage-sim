use std::fmt::Debug;

/// Controls how much information is recorded per storage operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionLogLevel {
    /// No transaction logging
    None,
    /// Log only transaction offsets and lengths
    Minimal,
    /// Log data for write transactions => fully reconstructible
    WriteDataOnly,
    /// Log data for read and write transactions
    ReadWriteData,
    /// Log all transaction data including data before erase transactions
    Full,
}

/// A recorded storage operation emitted by the simulator.
///
/// Each variant may carry optional data depending on the active
/// [`TransactionLogLevel`]. The generic parameter `O` is an optional
/// user-defined operation tag to help correlate high-level actions with
/// storage traffic.
#[derive(Debug, Clone)]
pub enum Transaction<O = ()> {
    Read {
        operation: Option<O>,
        offset: u32,
        length: usize,
        data: Option<Vec<u8>>,
    },
    Write {
        operation: Option<O>,
        offset: u32,
        data: Option<Vec<u8>>,
        after_write: Option<Vec<u8>>,
    },
    Erase {
        operation: Option<O>,
        from: u32,
        to: u32,
        data: Option<Vec<u8>>,
    },
}
impl<O> Transaction<O> {
    /// Construct a `Read` transaction based on the configured log level.
    ///
    /// When `level` is `ReadWriteData` or `Full`, the `data` buffer is captured.
    pub fn read(
        level: TransactionLogLevel,
        offset: u32,
        length: usize,
        data: &[u8],
        operation: Option<O>,
    ) -> Self {
        let data = match level {
            TransactionLogLevel::ReadWriteData | TransactionLogLevel::Full => Some(data.to_vec()),
            _ => None,
        };
        Transaction::Read {
            offset,
            length,
            data,
            operation,
        }
    }
    /// Construct a `Write` transaction based on the configured log level.
    ///
    /// The data being written is captured for `WriteDataOnly`, `ReadWriteData`
    /// and `Full`. The resulting post-write contents (`after_write`) are only
    /// captured for `Full`.
    pub fn write(
        level: TransactionLogLevel,
        offset: u32,
        data: &[u8],
        after_write: &[u8],
        operation: Option<O>,
    ) -> Self {
        let data = match level {
            TransactionLogLevel::WriteDataOnly
            | TransactionLogLevel::ReadWriteData
            | TransactionLogLevel::Full => Some(data.to_vec()),
            _ => None,
        };
        let after_write = match level {
            TransactionLogLevel::Full => Some(after_write.to_vec()),
            _ => None,
        };
        Transaction::Write {
            offset,
            data,
            after_write,
            operation,
        }
    }
    /// Construct an `Erase` transaction based on the configured log level.
    ///
    /// For `Full`, the pre-erase data over the erased range is captured.
    pub fn erase(
        level: TransactionLogLevel,
        from: u32,
        to: u32,
        data: &[u8],
        operation: Option<O>,
    ) -> Self {
        let data = match level {
            TransactionLogLevel::Full => Some(data.to_vec()),
            _ => None,
        };
        Transaction::Erase {
            from,
            to,
            data,
            operation,
        }
    }
}
