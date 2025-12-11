use embedded_storage_async::nor_flash::{NorFlash, ReadNorFlash};
use rand::SeedableRng;

#[derive(Clone, Copy, Debug)]
pub enum SpiType {
    SPI = 1,
    DSPI = 2,
    QSPI = 4,
}
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
    pub fn read_time(&self, total_bytes: usize, accesses: u32) -> fugit::NanosDurationU64 {
        self.read_time_per_byte * total_bytes as u32 + self.read_access_overhead * accesses
    }
    pub fn write_time(&self, total_bytes: usize, accesses: u32) -> fugit::NanosDurationU64 {
        self.write_time_per_byte * total_bytes as u32 + self.write_access_overhead * accesses
    }
    pub fn erase_time(&self, pages: usize, accesses: u32) -> fugit::MillisDurationU64 {
        self.page_erase_time * pages as u32 + (self.erase_access_overhead * accesses).convert()
    }
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

pub struct SimulatedNorFlashBuilder {
    size: usize,
    minimum_erase_cycles: u32,
    bit_failure_every_x_erases: u32,
    rng_seed: u64,
    log_level: TransactionLogLevel,
}
impl SimulatedNorFlashBuilder {
    pub fn new(size: usize) -> Self {
        Self {
            size,
            minimum_erase_cycles: u32::MAX,
            bit_failure_every_x_erases: u32::MAX,
            rng_seed: 0,
            log_level: TransactionLogLevel::None,
        }
    }
    pub fn with_minimum_erase_cycles(mut self, cycles: u32) -> Self {
        self.minimum_erase_cycles = cycles;
        self
    }
    pub fn with_failure_rate(mut self, bit_failure_every_x_erases: u32) -> Self {
        self.bit_failure_every_x_erases = bit_failure_every_x_erases;
        self
    }
    pub fn with_rng_seed(mut self, rng_seed: u64) -> Self {
        self.rng_seed = rng_seed;
        self
    }
    pub fn with_logging(mut self, level: TransactionLogLevel) -> Self {
        self.log_level = level;
        self
    }
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
    log_level: TransactionLogLevel,
    transactions: Vec<Transaction<O>>,
    rng: rand::rngs::SmallRng,
    minimum_safe_erase_cycles: u32,
    bit_failure_every_x_erases: u32,
    current_operation: Option<O>,
}

impl<O: Clone, const RS: usize, const WS: usize, const ES: usize> SimulatedNorFlash<O, RS, WS, ES> {
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
            log_level: TransactionLogLevel::None,
            transactions: Vec::new(),
            rng: rand::rngs::SmallRng::seed_from_u64(0),
            minimum_safe_erase_cycles: u32::MAX,
            bit_failure_every_x_erases: u32::MAX,
            current_operation: None,
        }
    }
    pub fn new_with_failures(
        size: usize,
        minimum_erase_cycles: u32,
        bit_failure_every_x_erases: u32,
        rng_seed: u64,
    ) -> Self {
        Self {
            minimum_safe_erase_cycles: minimum_erase_cycles,
            bit_failure_every_x_erases,
            rng: rand::rngs::SmallRng::seed_from_u64(rng_seed),
            ..Self::new(size)
        }
    }
    pub fn set_logging(&mut self, level: TransactionLogLevel) {
        self.log_level = level;
    }
    pub fn start_operation(&mut self, operation: Option<O>) {
        self.current_operation = operation;
    }
    pub fn reset_stats(&mut self) {
        self.read = 0;
        self.written = 0;
        self.erased = 0;
        self.read_accesses = 0;
        self.write_accesses = 0;
        self.erase_accesses = 0;
        self.transactions.clear();
        self.page_cycles.fill(0);
    }
    pub fn reset_failures(&mut self) {
        self.stuck_at_0_bits.fill(0);
        self.stuck_at_1_bits.fill(0);
        self.page_cycles.fill(0);
    }
    pub fn bytes_read(&self) -> usize {
        self.read
    }
    pub fn bytes_written(&self) -> usize {
        self.written
    }
    pub fn pages_erased(&self) -> usize {
        self.erased / Self::ERASE_SIZE
    }
    pub fn total_accesses(&self) -> usize {
        self.read_accesses + self.write_accesses + self.erase_accesses
    }
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
    pub fn transactions(&self) -> &[Transaction<O>] {
        &self.transactions
    }
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

pub type SimulatedNorFlashR1W1E4k<O> = SimulatedNorFlash<O, 1, 1, 4096>;
pub type SimulatedNorFlashR1W4E4k<O> = SimulatedNorFlash<O, 1, 4, 4096>;
pub type SimulatedNorFlashR4W4E4k<O> = SimulatedNorFlash<O, 4, 4, 4096>;

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
