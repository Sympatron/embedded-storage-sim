use std::convert::Infallible;

// use embedded_storage_sim::{FlashTimings, SimulatedNorFlashBuilder, TransactionLogLevel};
// use fugit::{ExtU64, RateExtU64};

mod gui;
mod workloads;

pub fn main() -> Result<(), sequential_storage::Error<Infallible>> {
    futures::executor::block_on(async_main())
}

async fn async_main() -> Result<(), sequential_storage::Error<Infallible>> {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "NOR Flash Simulator",
        native_options,
        Box::new(|cc| Ok(Box::new(gui::NorFlashApp::new(cc)))),
    )
    .unwrap();
    return Ok(());

    //     let flash_size = 1024 * 1024;
    //     let mut flash = SimulatedNorFlashBuilder::new(flash_size)
    //         .with_logging(TransactionLogLevel::WriteDataOnly)
    //         .build::<(), 1, 4, 4096>();
    //
    //     let config = sequential_storage::queue::QueueConfig::new(0..flash_size as u32);
    //     let cache = sequential_storage::cache::NoCache;
    //     let mut queue = sequential_storage::Storage::new_queue(&mut flash, config, cache);
    //
    //     let mut buf = [0; 128];
    //
    //     let mut count = 100000;
    //     const N: usize = 16;
    //     for i in 0..count {
    //         match queue.push(&[0xa5; N], false).await {
    //             Ok(()) => {}
    //             Err(e) => {
    //                 println!("Error after {} pushes: {:?}", i, e);
    //                 count = i;
    //                 break;
    //             }
    //         }
    //     }
    //     for _ in 0..count {
    //         let data = queue.pop(&mut buf).await?.unwrap();
    //         assert_eq!(data, &[0xa5; N]);
    //     }
    //
    //     let timings = FlashTimings::new(
    //         embedded_storage_sim::SpiType::QSPI,
    //         (125 / 2).MHz(),
    //         50.millis(),
    //         40,
    //     );
    //     let read_time = queue.flash().read_time(&timings);
    //     let write_time = queue.flash().write_time(&timings);
    //     let erase_time = queue.flash().erase_time(&timings);
    //     let total_time = queue.flash().total_time(&timings);
    //     println!(
    //         "Read: {}ms, Write: {}ms, Erase: {}ms, Total: {}ms",
    //         read_time.to_millis(),
    //         write_time.to_millis(),
    //         erase_time.to_millis(),
    //         total_time.to_millis()
    //     );
    //
    //     // for t in flash.transactions() {
    //     //     println!("{:?}", t);
    //     // }
    //     Ok(())
}
