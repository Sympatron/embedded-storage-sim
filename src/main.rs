use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use embedded_storage_sim::{SimulatedNorFlashBuilder, TransactionLogLevel};

pub fn main() {
    let mut flash = SimulatedNorFlashBuilder::new(1024 * 1024)
        .with_logging(TransactionLogLevel::WriteDataOnly)
        .build::<(), 1, 4, 4096>();
    flash.write(0, &[0xa5; 16]).unwrap();
    let mut buf = [0; 16];
    flash.read(0, &mut buf).unwrap();
    for t in flash.transactions() {
        println!("{:?}", t);
    }
}
