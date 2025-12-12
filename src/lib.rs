//! Embedded NOR flash simulator for experimenting with `embedded-storage` APIs.
//!
//! This crate provides a deterministic, in-memory NOR flash that implements
//! the `embedded-storage` and `embedded-storage-async` traits. It tracks
//! bytes read/written/erased, access counts, per-page erase cycles, and can
//! optionally log transactions for later inspection. You can also inject
//! simple failure models (stuck-at-0/1 bits after excessive erase cycles)
//! to emulate flash wear-out.
//!
//! Typical use:
//! - Build a flash with `SimulatedNorFlashBuilder` (optionally enabling
//!   logging and failure parameters).
//! - Use it anywhere a NOR flash implementing the storage traits is needed.
//! - Inspect statistics or compute timing estimates using `FlashTimings`.
//! - Capture a `FlashSnapshot` for UI or diagnostics.

use embedded_storage_async::nor_flash::{MultiwriteNorFlash, NorFlash, ReadNorFlash};
use rand::SeedableRng;

/// SPI line configuration used to derive effective bus throughput.
///
/// The numeric value corresponds to the number of data lines (lanes).
#[derive(Clone, Copy, Debug)]
pub enum SpiType {
    SPI = 1,
    DSPI = 2,
    QSPI = 4,
}
/// Helper to estimate operation durations for a given bus and device.
///
/// Construct via [`FlashTimings::new`] and pass it to the various `*_time`
/// methods on [`SimulatedNorFlash`] to get read/write/erase/total estimates.
#[derive(Clone, Copy, Debug)]
pub struct FlashTimings {
    read_time_per_byte: fugit::NanosDurationU64,
    write_time_per_byte: fugit::NanosDurationU64,
    page_erase_time: fugit::MillisDurationU64,
    read_access_overhead: fugit::NanosDurationU64,
    write_access_overhead: fugit::NanosDurationU64,
    erase_access_overhead: fugit::NanosDurationU64,
}

impl FlashTimings {
    /// Create timing parameters from bus type/frequency and device properties.
    ///
    /// - `spi_type`: Number of active data lanes (`SPI`/`DSPI`/`QSPI`).
    /// - `flash_frequency`: I/O clock frequency of the SPI bus.
    /// - `page_erase_time`: Typical erase duration for a single sector erase (typically 4 KiB).
    /// - `access_overhead_cycles`: Extra bus cycles per access (command, address, dummy cycles etc.).
    pub fn new(
        spi_type: SpiType,
        flash_frequency: fugit::MegahertzU64,
        page_erase_time: fugit::MillisDurationU64,
        access_overhead_cycles: u32,
    ) -> Self {
        let bytes_per_second = flash_frequency / (8 / spi_type as u32);
        let time_per_byte = fugit::NanosDurationU64::from_rate(bytes_per_second);
        let overhead_time =
            fugit::NanosDurationU64::from_rate(flash_frequency) * access_overhead_cycles;
        Self {
            read_time_per_byte: time_per_byte,
            write_time_per_byte: time_per_byte,
            page_erase_time,
            read_access_overhead: overhead_time,
            write_access_overhead: overhead_time,
            erase_access_overhead: overhead_time,
        }
    }
    /// Estimated read time for `total_bytes` over `accesses` logical operations.
    pub fn read_time(&self, total_bytes: usize, accesses: u32) -> fugit::NanosDurationU64 {
        self.read_time_per_byte * total_bytes as u32 + self.read_access_overhead * accesses
    }
    /// Estimated program time for `total_bytes` over `accesses` logical operations.
    pub fn write_time(&self, total_bytes: usize, accesses: u32) -> fugit::NanosDurationU64 {
        self.write_time_per_byte * total_bytes as u32 + self.write_access_overhead * accesses
    }
    /// Estimated erase time for `pages` erase units and `accesses` commands.
    pub fn erase_time(&self, pages: usize, accesses: u32) -> fugit::MillisDurationU64 {
        self.page_erase_time * pages as u32 + (self.erase_access_overhead * accesses).convert()
    }
    /// Combined estimate across reads, writes and erases.
    pub fn total_time(
        &self,
        read_bytes: usize,
        read_accesses: u32,
        write_bytes: usize,
        write_accesses: u32,
        erased_pages: usize,
        erase_accesses: u32,
    ) -> fugit::MillisDurationU64 {
        self.read_time(read_bytes, read_accesses).convert()
            + self.write_time(write_bytes, write_accesses).convert()
            + self.erase_time(erased_pages, erase_accesses)
    }
}

/// Builder for [`SimulatedNorFlash`], including logging and simple wear-out.
///
/// Use this when you want to tweak behavior (e.g. minimum safe erase cycles,
/// stuck-bit failure rate, deterministic RNG seed, or transaction log level).
pub struct SimulatedNorFlashBuilder {
    size: usize,
    minimum_erase_cycles: u32,
    bit_failure_every_x_erases: u32,
    rng_seed: Option<u64>,
    log_level: TransactionLogLevel,
}
impl SimulatedNorFlashBuilder {
    /// Start a builder for a flash of `size` bytes.
    pub fn new(size: usize) -> Self {
        Self {
            size,
            minimum_erase_cycles: u32::MAX,
            bit_failure_every_x_erases: u32::MAX,
            rng_seed: None,
            log_level: TransactionLogLevel::None,
        }
    }
    /// Set the maximum number of erase cycles considered "safe" per page.
    /// After this threshold, the simulator may introduce stuck-bit failures.
    pub fn with_minimum_erase_cycles(mut self, cycles: u32) -> Self {
        self.minimum_erase_cycles = cycles;
        self
    }
    /// Configure how frequently a stuck bit is injected past the safe limit.
    ///
    /// For example, `with_failure_rate(100)` creates one failure every 100
    /// erase cycles beyond [`with_minimum_erase_cycles`] for a given page.
    pub fn with_failure_rate(mut self, bit_failure_every_x_erases: u32) -> Self {
        self.bit_failure_every_x_erases = bit_failure_every_x_erases;
        self
    }
    /// Make failure injection deterministic by fixing the RNG seed.
    pub fn with_rng_seed(mut self, rng_seed: u64) -> Self {
        self.rng_seed = Some(rng_seed);
        self
    }
    /// Enable transaction logging at the requested granularity.
    pub fn with_logging(mut self, level: TransactionLogLevel) -> Self {
        self.log_level = level;
        self
    }
    /// Build a [`SimulatedNorFlash`] with the chosen `READ_SIZE`, `WRITE_SIZE`, and `ERASE_SIZE`.
    pub fn build<O: Clone, const RS: usize, const WS: usize, const ES: usize>(
        &self,
    ) -> SimulatedNorFlash<O, RS, WS, ES> {
        let mut flash = SimulatedNorFlash::new_with_failures(
            self.size,
            self.minimum_erase_cycles,
            self.bit_failure_every_x_erases,
            self.rng_seed,
        );
        flash.set_logging(self.log_level);
        flash
    }
}

/// In-memory NOR flash that implements the embedded storage traits.
///
/// Type parameters:
/// - `O`: Optional user-defined "operation" tag type stored alongside transactions.
/// - `READ_SIZE`: Minimum alignment for reads in bytes.
/// - `WRITE_SIZE`: Minimum alignment for writes in bytes.
/// - `ERASE_SIZE`: Erase unit size in bytes (also the page size for wear tracking).
pub struct SimulatedNorFlash<
    O = (),
    const READ_SIZE: usize = 1,
    const WRITE_SIZE: usize = 1,
    const ERASE_SIZE: usize = 4096,
> {
    data: Vec<u8>,
    stuck_at_1_bits: Vec<u8>,
    stuck_at_0_bits: Vec<u8>,
    page_cycles: Vec<u32>,
    read: usize,
    written: usize,
    erased: usize,
    read_accesses: usize,
    write_accesses: usize,
    erase_accesses: usize,
    total_operations: usize,
    log_level: TransactionLogLevel,
    transactions: Vec<Transaction<O>>,
    rng: rand::rngs::SmallRng,
    minimum_safe_erase_cycles: u32,
    bit_failure_every_x_erases: u32,
    current_operation: Option<O>,
}

impl<O: Clone, const RS: usize, const WS: usize, const ES: usize> SimulatedNorFlash<O, RS, WS, ES> {
    /// Create an erased flash (all bits set to 1) of `size` bytes.
    ///
    /// Panics if `size` is not a multiple of `ERASE_SIZE`.
    pub fn new(size: usize) -> Self {
        assert_eq!(0, size % Self::ERASE_SIZE);
        let page_count = size / Self::ERASE_SIZE;
        Self {
            data: vec![0xFF; size],
            stuck_at_1_bits: vec![0x00; size],
            stuck_at_0_bits: vec![0x00; size],
            page_cycles: vec![0; page_count],
            read: 0,
            written: 0,
            erased: 0,
            read_accesses: 0,
            write_accesses: 0,
            erase_accesses: 0,
            total_operations: 0,
            log_level: TransactionLogLevel::None,
            transactions: Vec::new(),
            rng: rand::rngs::SmallRng::seed_from_u64(0),
            minimum_safe_erase_cycles: u32::MAX,
            bit_failure_every_x_erases: u32::MAX,
            current_operation: None,
        }
    }
    /// Create a flash and configure failure model and RNG seed.
    ///
    /// Use this to simulate wear-out behavior without a separate builder.
    pub fn new_with_failures(
        size: usize,
        minimum_erase_cycles: u32,
        bit_failure_every_x_erases: u32,
        rng_seed: Option<u64>,
    ) -> Self {
        Self {
            minimum_safe_erase_cycles: minimum_erase_cycles,
            bit_failure_every_x_erases,
            rng: match rng_seed {
                Some(seed) => rand::rngs::SmallRng::seed_from_u64(seed),
                None => rand::rngs::SmallRng::from_os_rng(),
            },
            ..Self::new(size)
        }
    }
    /// Set the transaction logging level for subsequent operations.
    pub fn set_logging(&mut self, level: TransactionLogLevel) {
        self.log_level = level;
    }
    /// Attach an operation tag to the next transaction(s).
    ///
    /// Useful for correlating storage activity with high-level actions in
    /// higher layers. The tag is stored in emitted [`Transaction`]s.
    pub fn start_operation(&mut self, operation: O) {
        self.current_operation = Some(operation);
        self.total_operations += 1;
    }
    /// Erase all data and clear statistics and injected failures.
    pub fn reset(&mut self) {
        self.data.fill(0xFF);
        self.reset_stats();
        self.reset_failures();
    }
    /// Clear counters, transactions, and per-page erase cycle tracking.
    pub fn reset_stats(&mut self) {
        self.read = 0;
        self.written = 0;
        self.erased = 0;
        self.read_accesses = 0;
        self.write_accesses = 0;
        self.erase_accesses = 0;
        self.total_operations = 0;
        self.transactions.clear();
        self.page_cycles.fill(0);
        self.current_operation = None;
    }
    /// Remove all injected stuck-bit failures and reset wear counters.
    pub fn reset_failures(&mut self) {
        self.stuck_at_0_bits.fill(0);
        self.stuck_at_1_bits.fill(0);
        self.page_cycles.fill(0);
    }
    /// Total flash capacity in bytes.
    pub fn size(&self) -> usize {
        self.data.len()
    }
    /// Number of erase units (pages) in the flash.
    pub fn page_count(&self) -> usize {
        self.page_cycles.len()
    }
    /// Total amount of bytes read since last stats reset.
    pub fn bytes_read(&self) -> usize {
        self.read
    }
    /// Total amount of bytes written since last stats reset.
    pub fn bytes_written(&self) -> usize {
        self.written
    }
    /// Number of erase units erased since last stats reset.
    pub fn pages_erased(&self) -> usize {
        self.erased / Self::ERASE_SIZE
    }
    /// Number of times [`start_operation`] was called.
    pub fn total_operations(&self) -> usize {
        self.total_operations
    }
    /// Total number of storage accesses (reads + writes + erases).
    pub fn total_accesses(&self) -> usize {
        self.read_accesses + self.write_accesses + self.erase_accesses
    }
    /// Estimate the time spent reading based on accumulated stats.
    pub fn read_time(&self, timings: &FlashTimings) -> fugit::NanosDurationU64 {
        timings.read_time(self.read, self.read_accesses as u32)
    }
    /// Estimate the time spent programming based on accumulated stats.
    pub fn write_time(&self, timings: &FlashTimings) -> fugit::NanosDurationU64 {
        timings.write_time(self.written, self.write_accesses as u32)
    }
    /// Estimate the time spent erasing based on accumulated stats.
    pub fn erase_time(&self, timings: &FlashTimings) -> fugit::MillisDurationU64 {
        timings.erase_time(self.erased / Self::ERASE_SIZE, self.erase_accesses as u32)
    }
    /// Estimate total time across all operations based on stats.
    pub fn total_time(&self, timings: &FlashTimings) -> fugit::MillisDurationU64 {
        timings.total_time(
            self.read,
            self.read_accesses as u32,
            self.written,
            self.write_accesses as u32,
            self.erased / Self::ERASE_SIZE,
            self.erase_accesses as u32,
        )
    }
    /// View the recorded transaction log.
    pub fn transactions(&self) -> &[Transaction<O>] {
        &self.transactions
    }
    /// Per-page erase cycle counters for wear analysis.
    pub fn page_erase_cycles(&self) -> &[u32] {
        &self.page_cycles
    }
}

mod blocking;
mod transaction;
pub use transaction::{Transaction, TransactionLogLevel};

impl<O: Clone, const RS: usize, const WS: usize, const ES: usize> ReadNorFlash
    for SimulatedNorFlash<O, RS, WS, ES>
{
    const READ_SIZE: usize = RS;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        embedded_storage::nor_flash::ReadNorFlash::read(self, offset, bytes)
    }

    fn capacity(&self) -> usize {
        self.data.len()
    }
}
impl<O: Clone, const RS: usize, const WS: usize, const ES: usize> NorFlash
    for SimulatedNorFlash<O, RS, WS, ES>
{
    const WRITE_SIZE: usize = WS;
    const ERASE_SIZE: usize = ES;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        embedded_storage::nor_flash::NorFlash::erase(self, from, to)
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        embedded_storage::nor_flash::NorFlash::write(self, offset, bytes)
    }
}

impl<O: Clone, const RS: usize, const WS: usize, const ES: usize> MultiwriteNorFlash
    for SimulatedNorFlash<O, RS, WS, ES>
{
}
impl<O: Clone, const RS: usize, const WS: usize, const ES: usize> MultiwriteNorFlash
    for &mut SimulatedNorFlash<O, RS, WS, ES>
{
}

/// Convenience alias: read alignment 1B, write alignment 1B, erase alignment 4KiB.
pub type SimulatedNorFlashR1W1E4k<O> = SimulatedNorFlash<O, 1, 1, 4096>;
/// Convenience alias: read alignment 1B, write alignment 4B, erase alignment 4KiB (common for NOR flashes).
pub type SimulatedNorFlashR1W4E4k<O> = SimulatedNorFlash<O, 1, 4, 4096>;
/// Convenience alias: read alignment 4B, write alignment 4B, erase alignment 4KiB.
pub type SimulatedNorFlashR4W4E4k<O> = SimulatedNorFlash<O, 4, 4, 4096>;

/// Common dynamic wrapper for typical `READ_SIZE`/`WRITE_SIZE`/`ERASE_SIZE` combos.
///
/// Use `From` to convert a specific configuration into this enum when you need
/// a single type to hold different `SimulatedNorFlash` configurations.
pub enum AnySimulatedNorFlash<O = ()> {
    R1W1E4k(SimulatedNorFlashR1W1E4k<O>),
    R1W4E4k(SimulatedNorFlashR1W4E4k<O>),
    R4W4E4k(SimulatedNorFlashR4W4E4k<O>),
}

impl<O> From<SimulatedNorFlashR1W1E4k<O>> for AnySimulatedNorFlash<O> {
    fn from(flash: SimulatedNorFlashR1W1E4k<O>) -> Self {
        AnySimulatedNorFlash::R1W1E4k(flash)
    }
}
impl<O> From<SimulatedNorFlashR1W4E4k<O>> for AnySimulatedNorFlash<O> {
    fn from(flash: SimulatedNorFlashR1W4E4k<O>) -> Self {
        AnySimulatedNorFlash::R1W4E4k(flash)
    }
}
impl<O> From<SimulatedNorFlashR4W4E4k<O>> for AnySimulatedNorFlash<O> {
    fn from(flash: SimulatedNorFlashR4W4E4k<O>) -> Self {
        AnySimulatedNorFlash::R4W4E4k(flash)
    }
}

/// A lightweight capture of the flash state and statistics for inspection.
#[derive(Clone, Default, Debug)]
pub struct FlashSnapshot {
    /// Full raw contents, if requested via [`SimulatedNorFlash::snapshot`].
    pub data: Option<Vec<u8>>,
    /// Per-page erase cycle counters.
    pub page_cycles: Vec<u32>,
    /// Amount of bytes read so far.
    pub bytes_read: usize,
    /// Amount of bytes written so far.
    pub bytes_written: usize,
    /// Number of erase units erased so far.
    pub pages_erased: usize,
    /// Total number of accesses (read+write+erase).
    pub total_accesses: usize,
    /// Number of times a high level operation was started.
    pub total_operations: usize,
    /// Number of entries in the transaction log.
    pub transactions_len: usize,
    /// The most recent operation tag, if any.
    pub last_operation: Option<String>,
}

impl<O: Clone + ToString, const RS: usize, const WS: usize, const ES: usize>
    SimulatedNorFlash<O, RS, WS, ES>
{
    /// Create a [`FlashSnapshot`]. When `with_data` is `true`, includes contents.
    pub fn snapshot(&self, with_data: bool) -> FlashSnapshot {
        FlashSnapshot {
            data: if with_data {
                Some(self.data.clone())
            } else {
                None
            },
            page_cycles: self.page_erase_cycles().to_vec(),
            bytes_read: self.bytes_read(),
            bytes_written: self.bytes_written(),
            pages_erased: self.pages_erased(),
            total_accesses: self.total_accesses(),
            total_operations: self.total_operations(),
            transactions_len: self.transactions().len(),
            last_operation: self.current_operation.as_ref().map(|op| op.to_string()),
        }
    }
}
