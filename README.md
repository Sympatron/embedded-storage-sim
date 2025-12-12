# Embedded storage simulation framework

> [!WARNING]
> This project is experimental and not production-ready.
>
> - APIs may change without notice.
> - Use at your own risk.


A Rust project to simulate NOR flash behavior for experimentation and testing. It consists of:
- A library crate providing a configurable NOR flash simulator.
- A WIP GUI to visualize operations and page erase cycles.

## Features

- Simulated NOR flash with:
    - Configurable read/write/erase granularities.
    - Erase cycle tracking per page.
    - Optional bit failure injection after configurable amount of erase cycles.
    - Transaction logging with configurable details (None → Full).
    - Snapshots for statistics and visualization.
- Async-/embedded-storage compatible traits (ReadNorFlash, NorFlash, MultiwriteNorFlash).
- GUI (eframe/egui) to visualize stats and page wear.

## Quick start (library)

Add as a local crate or use path dependency. Example:

```rust
use embedded_storage_sim::{SimulatedNorFlashBuilder, TransactionLogLevel};

let flash_size = 256 * 1024;
let mut flash = SimulatedNorFlashBuilder::new(flash_size)
        .with_minimum_erase_cycles(100)
        .with_failure_rate(10) // introduce bit failures every N cycles past minimum
        .with_logging(TransactionLogLevel::Minimal)
        .build::<(), 1, 4, 4096>(); // READ_SIZE=1, WRITE_SIZE=4, ERASE_SIZE=4096

// TODO Use with sequential-storage or directly via embedded-storage traits.
let snapshot = flash.snapshot(false);
```


`SimulatedNorFlash` implements `embedded_storage(_async)::nor_flash::{ReadNorFlash, NorFlash, MultiwriteNorFlash}`

## GUI (WIP)

Run the GUI to visualize operations and page cycles:

```bash
cargo run
```

Use buttons to start workloads. The GUI will show stats and page wear in real time.

## Workloads

Includes example workloads for [`sequential-storage`](https://github.com/tweedegolf/sequential-storage):
- [`Queue`](https://docs.rs/sequential-storage/latest/sequential_storage/queue/index.html): push until full, then pop all.
- [`Map`](https://docs.rs/sequential-storage/latest/sequential_storage/map/index.html): store random keys and remove them.

## License

This project is licensed under the BSD 3-Clause License.

Copyright (c) 2025 Sympatron GmbH. See the LICENSE file for the full text.