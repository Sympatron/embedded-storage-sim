use std::fmt::Debug;

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
