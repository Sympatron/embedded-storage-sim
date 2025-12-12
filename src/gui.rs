use eframe::egui;
use embedded_storage_sim::{FlashSnapshot, SimulatedNorFlashBuilder, TransactionLogLevel};
use std::{
    panic::catch_unwind,
    sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
    thread,
};

use crate::workloads::{self, sequential_storage::queue::queue_push_full_and_pop_all};

pub struct NorFlashApp {
    shared: Arc<Mutex<Option<FlashSnapshot>>>,
    runner: Runner,
    // cancel_worker: Arc<AtomicBool>,
    // running_workload: Arc<AtomicBool>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Workload {
    Stop,
    Cancel,
    Reset,
    SqQueue,
    SqMap,
}

pub struct Runner {
    workload_rx: Option<mpsc::Receiver<Workload>>,
    workload_tx: mpsc::Sender<Workload>,
    is_working: Arc<AtomicBool>,
}

impl Runner {
    pub fn new() -> Self {
        let (workload_tx, workload_rx) = mpsc::channel();
        Self {
            workload_rx: Some(workload_rx),
            workload_tx,
            is_working: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start_workload(&self, _workload: Workload) {
        self.workload_tx.send(_workload).ok();
    }

    fn do_run(
        rx: Arc<Mutex<mpsc::Receiver<Workload>>>,
        shared_snapshot: Arc<Mutex<Option<FlashSnapshot>>>,
        is_running: Arc<AtomicBool>,
    ) -> Result<(), ()> {
        let flash_size = 256 * 1024;
        let mut flash = SimulatedNorFlashBuilder::new(flash_size)
            .with_logging(TransactionLogLevel::None)
            .build::<_, 1, 4, 4096>();

        let mut next_workload = None;
        loop {
            let mut workload = {
                if let Some(workload) = next_workload.take() {
                    workload
                } else {
                    let rx_lock = rx.lock().unwrap();
                    match rx_lock.recv() {
                        Ok(w) => w,
                        Err(_) => {
                            return Ok(()); // channel closed
                        }
                    }
                }
            };
            while let Some(next) = rx.lock().unwrap().try_recv().ok() {
                // drain to latest
                workload = next;
            }
            is_running.store(true, std::sync::atomic::Ordering::SeqCst);
            match workload {
                Workload::Stop => {
                    return Ok(());
                }
                Workload::Cancel => {}
                Workload::Reset => {
                    flash.reset();
                    let snapshot = flash.snapshot(false);
                    if let Ok(mut guard) = shared_snapshot.lock() {
                        *guard = Some(snapshot); // overwrite latest
                    }
                }
                Workload::SqQueue => {
                    // run your async workload via block_on or a runtime
                    futures::executor::block_on(queue_push_full_and_pop_all(&mut flash, |flash| {
                        let snapshot = flash.snapshot(false);
                        if let Ok(mut guard) = shared_snapshot.lock() {
                            *guard = Some(snapshot); // overwrite latest
                        }
                        next_workload = rx.lock().unwrap().try_recv().ok();
                        next_workload.is_none()
                    }))
                    .ok();
                }
                Workload::SqMap => {
                    // run your async workload via block_on or a runtime
                    futures::executor::block_on(workloads::sequential_storage::map::map_fill(
                        &mut flash,
                        |flash| {
                            let snapshot = flash.snapshot(false);
                            if let Ok(mut guard) = shared_snapshot.lock() {
                                *guard = Some(snapshot); // overwrite latest
                            }
                            next_workload = rx.lock().unwrap().try_recv().ok();
                            next_workload.is_none()
                        },
                    ))
                    .ok();
                }
            }
            is_running.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }

    pub fn run(&mut self, shared_snapshot: Arc<Mutex<Option<FlashSnapshot>>>) {
        let rx: Arc<Mutex<mpsc::Receiver<Workload>>> =
            Arc::new(Mutex::new(self.workload_rx.take().unwrap()));
        let is_working: Arc<AtomicBool> = Arc::clone(&self.is_working);
        thread::spawn(move || {
            loop {
                let rx = Arc::clone(&rx);
                let shared_snapshot = Arc::clone(&shared_snapshot);
                let is_working_clone = Arc::clone(&is_working);
                if catch_unwind(move || {
                    Self::do_run(rx, shared_snapshot, Arc::clone(&is_working_clone))
                })
                .is_ok()
                {
                    break;
                }
                is_working.store(false, std::sync::atomic::Ordering::SeqCst);
                eprintln!("Workload runner panicked, restarting...");
            }
        });
    }

    pub fn is_working(&self) -> bool {
        self.is_working.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl NorFlashApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let shared = Arc::new(Mutex::new(None));
        let mut runner = Runner::new();
        runner.run(Arc::clone(&shared));

        Self { shared, runner }
    }
}

fn human_readable_bytes(bytes: usize) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;
    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }
    format!("{:.2} {}", size, UNITS[unit_index])
}

impl eframe::App for NorFlashApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Receive pending snapshots (non-blocking)
        let snapshot = {
            // take a clone of current state
            let guard = self.shared.lock().unwrap();
            guard.clone()
        };

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            if self.runner.is_working() {
                ui.heading("NOR Flash Simulator 🚀");
            } else {
                ui.heading("NOR Flash Simulator");
            }
            ui.horizontal(|ui| {
                if ui.button("Reset").clicked() {
                    self.runner.start_workload(Workload::Reset);
                }
                if ui.button("Run queue workload").clicked() {
                    self.runner.start_workload(Workload::SqQueue);
                }
                if ui.button("Run map workload").clicked() {
                    self.runner.start_workload(Workload::SqMap);
                }
                if self.runner.is_working() {
                    if ui.button("Cancel").clicked() {
                        self.runner.start_workload(Workload::Cancel);
                    }
                }
            });

            if let Some(s) = &snapshot {
                ui.label(format!(
                    "Last operation: {}",
                    s.last_operation.as_deref().unwrap_or("")
                ));
                ui.label(format!("Total ops: {}", s.total_operations));
                ui.label(format!(
                    "Bytes read: {}",
                    human_readable_bytes(s.bytes_read)
                ));
                ui.label(format!(
                    "Bytes written: {}",
                    human_readable_bytes(s.bytes_written)
                ));
                ui.label(format!("Pages erased: {}", s.pages_erased));
                ui.label(format!("Total accesses: {}", s.total_accesses));
                // ui.label(format!("Transactions: {}", s.transactions_len));
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Page erase cycles");

            if let Some(snapshot) = &snapshot {
                draw_page_grid(ui, snapshot);
            } else {
                ui.label("Waiting for first snapshot...");
            }
        });

        ctx.request_repaint(); // continuous repaint to show new snapshots
    }
}

fn draw_page_grid(ui: &mut egui::Ui, snapshot: &FlashSnapshot) {
    let page_cycles = &snapshot.page_cycles;
    if page_cycles.is_empty() {
        ui.label("No pages");
        return;
    }

    // For now, lay out as a simple square-ish grid.
    let pages = page_cycles.len();
    let cols = (pages as f32).sqrt().ceil() as usize;
    let rows = (pages + cols - 1) / cols;

    let cell_size = egui::vec2(30.0, 30.0);
    let red_cycle_count = page_cycles.iter().copied().max().unwrap_or(100).max(20); // cycles at which cell is fully red

    egui::Grid::new("page_grid")
        .min_col_width(cell_size.x) // override default ~40
        .min_row_height(cell_size.y) // override default interact_size.y
        .max_col_width(cell_size.x) // clamp to exactly 30
        .spacing(egui::vec2(1.0, 1.0)) // 1px-ish spacing between cells
        .show(ui, |ui| {
            ui.set_width(30.0 * cols as f32);
            let mut idx = 0;
            for _row in 0..rows {
                for _col in 0..cols {
                    if idx >= pages {
                        // ui.label(""); // filler
                        // filler cell so end_row keeps layout correct
                        ui.allocate_exact_size(cell_size, egui::Sense::hover());
                    } else {
                        let cycles = page_cycles[idx];
                        let norm =
                            (cycles.min(red_cycle_count) as f32) / red_cycle_count.max(1) as f32;
                        let intensity = (norm * 255.0) as u8;

                        let color = egui::Color32::from_rgb(intensity, 255 - intensity, 0);

                        egui::Frame::NONE
                            .fill(color)
                            .outer_margin(0.0)
                            .inner_margin(0.0)
                            .show(ui, |ui| {
                                ui.set_min_size(cell_size);
                                // Force the content area to 30×30
                                ui.allocate_exact_size(cell_size, egui::Sense::hover());
                                // ui.take_available_space();
                            });

                        idx += 1;
                    }
                }
                ui.end_row();
            }
        });
}
