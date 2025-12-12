use embedded_storage_sim::SimulatedNorFlash;
use futures::future::LocalBoxFuture;
use rand::Rng;

use crate::workloads::Operation;

pub fn map_fill<'a, F, const RS: usize, const WS: usize, const ES: usize>(
    flash: &'a mut SimulatedNorFlash<Operation, RS, WS, ES>,
    mut hook: F,
) -> LocalBoxFuture<'a, anyhow::Result<()>>
where
    F: FnMut(&SimulatedNorFlash<Operation, RS, WS, ES>) -> bool + 'a,
{
    Box::pin(async move {
        let flash_size = flash.size();
        let config = sequential_storage::map::MapConfig::new(0..flash_size as u32);
        let cache = sequential_storage::cache::NoCache;
        let mut map = sequential_storage::Storage::new_map(flash, config, cache);

        let mut buf = [0; 128];

        let count = 10000;
        const N: usize = 16;
        let mut rng = rand::rng();

        for _ in 0..10 {
            let mut stored_keys = Vec::new();
            for i in 0..count {
                let key = rng.random_range(0..count);
                map.flash().start_operation(Operation::MapStore);
                match map.store_item(&mut buf, &key, &[0xa5; N]).await {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("Full after {} stores: {:?}", i, e);
                        break;
                    }
                }
                stored_keys.push(key);

                if !hook(map.flash()) {
                    return Ok(());
                }
            }

            for key in stored_keys {
                map.flash().start_operation(Operation::MapRemove);
                map.remove_item(&mut buf, &key).await?;

                if !hook(map.flash()) {
                    return Ok(());
                }
            }
        }

        Ok(())
    })
}
