use embedded_storage_sim::{FlashStats, SimulatedNorFlash};
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

#[derive(Debug, Default)]
pub struct MapBenchmarkResult {
    pub sequential_overwrite_stats: Option<FlashStats>,
    pub sequential_overwrite_ops: Vec<FlashStats>,
    pub random_overwrite_stats: Option<FlashStats>,
    pub random_overwrite_ops: Vec<FlashStats>,
}

pub fn benchmark_map_store<'a, F, const RS: usize, const WS: usize, const ES: usize>(
    flash: &'a mut SimulatedNorFlash<Operation, RS, WS, ES>,
    cache: impl sequential_storage::cache::KeyCacheImpl<i32> + 'a,
    rng: &'a mut impl Rng,
    random_count: usize,
    mut hook: F,
) -> LocalBoxFuture<'a, anyhow::Result<MapBenchmarkResult>>
where
    F: FnMut(&SimulatedNorFlash<Operation, RS, WS, ES>) -> bool + 'a,
{
    Box::pin(async move {
        let flash_size = flash.size();
        let config = sequential_storage::map::MapConfig::new(0..flash_size as u32);
        let mut map = sequential_storage::Storage::new_map(flash, config, cache);

        let mut buf = [0; 128];

        // fill the map
        let mut key = 0;
        map.flash().start_operation(Operation::MapStore);
        while map.store_item(&mut buf, &key, &[0xa5; 16]).await.is_ok() {
            key += 1;
            if !hook(map.flash()) {
                return Ok(Default::default());
            }
            map.flash().start_operation(Operation::MapStore);
        }
        eprintln!("Map filled with {} items", key);

        let count = key;
        let items_per_page = count / map.flash().page_count() as i32;
        let count = count - items_per_page * 2;
        // let (flash, cache) = map.destroy();
        // flash.reset();
        // let config = sequential_storage::map::MapConfig::new(0..flash_size as u32);
        // let mut map = sequential_storage::Storage::new_map(flash, config, cache);
        map.flash().reset();

        // Fill again, but leave at least 2 pages free
        map.flash().start_operation(Operation::MapStore);
        for key in 0..count {
            map.store_item(&mut buf, &key, &[0xa5; 16]).await?;

            if !hook(map.flash()) {
                return Ok(Default::default());
            }
            map.flash().start_operation(Operation::MapStore);
        }
        eprintln!("Map filled with {} items", count);

        // benchmark overwriting existing keys sequentially
        map.flash().reset_stats();
        let mut op_stats = Vec::new();
        for key in 0..count {
            map.flash().start_operation(Operation::MapStore);
            map.store_item(&mut buf, &key, &[0xa5; 16]).await?;
            op_stats.push(map.flash().last_operation_stats());

            if !hook(map.flash()) {
                return Ok(Default::default());
            }
        }
        let sequential_overwrite_stats = map.flash().stats();
        let sequential_overwrite_ops = op_stats;

        // benchmark overwriting existing keys randomly
        map.flash().reset_stats();
        let mut op_stats = Vec::new();
        for _ in 0..random_count {
            let key = rng.random_range(0..count);
            map.flash().start_operation(Operation::MapStore);
            map.store_item(&mut buf, &key, &[0xa5; 16]).await?;
            op_stats.push(map.flash().last_operation_stats());

            if !hook(map.flash()) {
                return Ok(Default::default());
            }
        }
        let random_overwrite_stats = map.flash().stats();
        let random_overwrite_ops = op_stats;

        Ok(MapBenchmarkResult {
            sequential_overwrite_stats: Some(sequential_overwrite_stats),
            sequential_overwrite_ops,
            random_overwrite_stats: Some(random_overwrite_stats),
            random_overwrite_ops,
        })
    })
}
