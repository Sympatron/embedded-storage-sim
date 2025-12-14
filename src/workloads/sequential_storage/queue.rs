use embedded_storage_sim::SimulatedNorFlash;
use futures::future::BoxFuture;

use crate::workloads::Operation;

pub fn queue_push_full_and_pop_all<'a, F, const RS: usize, const WS: usize, const ES: usize>(
    flash: &'a mut SimulatedNorFlash<Operation, RS, WS, ES>,
    run_count: usize,
    mut hook: F,
) -> BoxFuture<'a, anyhow::Result<()>>
where
    F: FnMut(&SimulatedNorFlash<Operation, RS, WS, ES>) -> bool + Send + 'a,
{
    Box::pin(async move {
        let flash_size = flash.size();
        let config = sequential_storage::queue::QueueConfig::new(0..flash_size as u32);
        let cache = sequential_storage::cache::NoCache;
        let mut queue = sequential_storage::Storage::new_queue(flash, config, cache);

        let mut buf = [0; 128];

        const N: usize = 16;

        for _ in 0..run_count {
            let mut count = 100000;
            for i in 0..count {
                queue.flash().start_operation(Operation::QueuePush);
                match queue.push(&[0xa5; N], false).await {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("Full after {} pushes: {:?}", i, e);
                        count = i;
                        break;
                    }
                }

                // Call the hook on every push, or every N pushes.
                if !hook(queue.flash()) {
                    return Ok(());
                }
            }

            for _ in 0..count {
                queue.flash().start_operation(Operation::QueuePop);
                let data = queue.pop(&mut buf).await?.unwrap();
                assert_eq!(data, &[0xa5; N]);

                // Optionally hook on pops as well.
                if !hook(queue.flash()) {
                    return Ok(());
                }
            }
        }

        Ok(())
    })
}
