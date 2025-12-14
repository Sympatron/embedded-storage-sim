#![allow(unused_imports, dead_code)]
use eframe::egui::{self, FontId, InnerResponse, Label, Response, ahash::HashMap};
use egui_alignments::Alignable;
use egui_plot::{
    AxisHints, Bar, BarChart, CoordinatesFormatter, Corner, HLine, HPlacement, Legend, Line, Plot,
    PlotPoints, VPlacement,
};
use egui_tiles::{Tabs, TileId};
use embedded_storage_sim::{FlashSnapshot, SimulatedNorFlashBuilder, TransactionLogLevel};
use fugit::NanosDurationU64;
use std::{
    fmt::Display,
    hash::Hash,
    panic::catch_unwind,
    sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
    thread,
};

use crate::workloads::{self, sequential_storage::queue::queue_push_full_and_pop_all};

#[derive(Default)]
pub struct AppState {
    pub run_data: HashMap<String, RunData>,
    pub new_snapshot: Option<FlashSnapshot>,
    pub snapshot: Option<FlashSnapshot>,
}

pub struct NorFlashApp {
    tree: egui_tiles::Tree<TabPane>,
    plot_tabs: HashMap<String, TileId>,
    runner: Runner,
    state: Arc<Mutex<AppState>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Workload {
    #[allow(dead_code)]
    Stop,
    Cancel,
    Reset,
    SqQueue,
    SqMap,
    SqBenchmarkMapStore,
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
        app_state: Arc<Mutex<AppState>>,
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
                    if let Ok(mut guard) = app_state.lock() {
                        guard.new_snapshot = Some(snapshot); // overwrite latest
                    }
                }
                Workload::SqQueue => {
                    // run your async workload via block_on or a runtime
                    futures::executor::block_on(queue_push_full_and_pop_all(
                        &mut flash,
                        101,
                        |flash| {
                            let snapshot = flash.snapshot(false);
                            if let Ok(mut guard) = app_state.lock() {
                                guard.new_snapshot = Some(snapshot); // overwrite latest
                            }
                            next_workload = rx.lock().unwrap().try_recv().ok();
                            next_workload.is_none()
                        },
                    ))
                    .ok();
                }
                Workload::SqMap => {
                    // run your async workload via block_on or a runtime
                    futures::executor::block_on(workloads::sequential_storage::map::map_fill(
                        &mut flash,
                        |flash| {
                            let snapshot = flash.snapshot(false);
                            if let Ok(mut guard) = app_state.lock() {
                                guard.new_snapshot = Some(snapshot); // overwrite latest
                            }
                            next_workload = rx.lock().unwrap().try_recv().ok();
                            next_workload.is_none()
                        },
                    ))
                    .ok();
                }
                Workload::SqBenchmarkMapStore => {
                    // run your async workload via block_on or a runtime
                    match futures::executor::block_on(
                        workloads::sequential_storage::map::benchmark_map_store(
                            &mut flash,
                            sequential_storage::cache::NoCache,
                            &mut rand::rng(),
                            300,
                            |flash| {
                                let snapshot = flash.snapshot(false);
                                if let Ok(mut guard) = app_state.lock() {
                                    guard.new_snapshot = Some(snapshot); // overwrite latest
                                }
                                next_workload = rx.lock().unwrap().try_recv().ok();
                                next_workload.is_none()
                            },
                        ),
                    ) {
                        Ok(result) => {
                            let timings = embedded_storage_sim::FlashTimings::new(
                                embedded_storage_sim::SpiType::QSPI,
                                fugit::RateExtU64::MHz(125 / 2),
                                fugit::MillisDurationU64::millis(50),
                                40,
                            );
                            // println!("Benchmark result: {:#?}", result);
                            if let Some(stats) = result.sequential_overwrite_stats {
                                let data_stats = print_benchmark_results(
                                    "Sequential overwrite",
                                    &timings,
                                    &result.sequential_overwrite_ops,
                                    &stats,
                                );
                                let mut app_state = app_state.lock().unwrap();
                                app_state
                                    .run_data
                                    .insert("Sequential overwrite".to_string(), data_stats);
                            }
                            if let Some(stats) = result.random_overwrite_stats {
                                let data_stats = print_benchmark_results(
                                    "Random overwrite",
                                    &timings,
                                    &result.random_overwrite_ops,
                                    &stats,
                                );
                                let mut app_state = app_state.lock().unwrap();
                                app_state
                                    .run_data
                                    .insert("Random overwrite".to_string(), data_stats);
                            }
                        }
                        Err(e) => {
                            eprintln!("Benchmark error: {:?}", e);
                        }
                    }
                }
            }
            is_running.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }

    pub fn run(&mut self, app_state: Arc<Mutex<AppState>>) {
        let rx: Arc<Mutex<mpsc::Receiver<Workload>>> =
            Arc::new(Mutex::new(self.workload_rx.take().unwrap()));
        let is_working: Arc<AtomicBool> = Arc::clone(&self.is_working);
        thread::spawn(move || {
            loop {
                let rx = Arc::clone(&rx);
                let app_state_clone = Arc::clone(&app_state);
                let is_working_clone = Arc::clone(&is_working);
                if catch_unwind(move || {
                    Self::do_run(
                        rx,
                        Arc::clone(&app_state_clone),
                        Arc::clone(&is_working_clone),
                    )
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

pub fn median<T: Clone>(sorted_data: &[T], default: T) -> T {
    let len = sorted_data.len();
    if len == 0 {
        default
    } else if len % 2 == 1 {
        sorted_data[len / 2].clone()
    } else {
        sorted_data[len / 2 - 1].clone() // lower median
    }
}
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RunData {
    pub data: Vec<NanosDurationU64>,
    pub min: NanosDurationU64,
    pub max: NanosDurationU64,
    pub avg: NanosDurationU64,
    pub std_dev: NanosDurationU64,
    pub median: NanosDurationU64,
    pub p90: NanosDurationU64,
    pub p99: NanosDurationU64,
    pub p999: NanosDurationU64,
    pub total: NanosDurationU64,
    pub count: usize,
}
impl Default for RunData {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            min: NanosDurationU64::from_ticks(0),
            max: NanosDurationU64::from_ticks(0),
            avg: NanosDurationU64::from_ticks(0),
            std_dev: NanosDurationU64::from_ticks(0),
            median: NanosDurationU64::from_ticks(0),
            p90: NanosDurationU64::from_ticks(0),
            p99: NanosDurationU64::from_ticks(0),
            p999: NanosDurationU64::from_ticks(0),
            total: NanosDurationU64::from_ticks(0),
            count: 0,
        }
    }
}
impl From<&[NanosDurationU64]> for RunData {
    fn from(data: &[NanosDurationU64]) -> Self {
        let count = data.len();
        if count == 0 {
            return Self {
                data: Vec::new(),
                min: NanosDurationU64::from_ticks(0),
                max: NanosDurationU64::from_ticks(0),
                avg: NanosDurationU64::from_ticks(0),
                std_dev: NanosDurationU64::from_ticks(0),
                median: NanosDurationU64::from_ticks(0),
                p90: NanosDurationU64::from_ticks(0),
                p99: NanosDurationU64::from_ticks(0),
                p999: NanosDurationU64::from_ticks(0),
                total: NanosDurationU64::from_ticks(0),
                count: 0,
            };
        }
        let mut sorted_data: Vec<_> = data.to_vec();
        sorted_data.sort();
        let total = sorted_data
            .iter()
            .fold(NanosDurationU64::from_ticks(0), |acc, &t| acc + t);
        let avg = total / (count as u32);
        let std_dev = {
            let mean = avg;
            let var = sorted_data.iter().fold(0, |acc, &t| {
                let diff = t.ticks() as i64 - mean.ticks() as i64;
                acc + (diff * diff) as u64
            }) / (count as u64);
            NanosDurationU64::from_ticks((var as f64).sqrt() as u64)
        };
        Self {
            data: data.to_vec(),
            min: sorted_data[0],
            max: *sorted_data.last().unwrap(),
            avg,
            std_dev,
            median: median(&sorted_data, NanosDurationU64::from_ticks(0)),
            p90: sorted_data[(count as f32 * 0.9).ceil() as usize - 1],
            p99: sorted_data[(count as f32 * 0.99).ceil() as usize - 1],
            p999: sorted_data[(count as f32 * 0.999).ceil() as usize - 1],
            total,
            count,
        }
    }
}
fn print_benchmark_results(
    name: &str,
    timings: &embedded_storage_sim::FlashTimings,
    op_stats: &[embedded_storage_sim::FlashStats],
    _overall_stats: &embedded_storage_sim::FlashStats,
) -> RunData {
    // let mut op_times: Vec<_> = op_stats.iter().map(|s| timings.total_time(s)).collect();
    // // println!(
    // //     "times: {:?}",
    // //     op_times.iter().map(|t| t.to_micros()).collect::<Vec<_>>()
    // // );
    // op_times.sort();
    // let avg_op_time = op_times
    //     .iter()
    //     .fold(NanosDurationU64::from_ticks(0), |acc, &t| acc + t)
    //     / (op_times.len() as u32);
    // let std_dev_op_time = {
    //     let mean = avg_op_time;
    //     let var = op_times.iter().fold(0, |acc, &t| {
    //         let diff = t.ticks() as i64 - mean.ticks() as i64;
    //         acc + (diff * diff) as u64
    //     }) / (op_times.len() as u64);
    //     NanosDurationU64::from_ticks((var as f64).sqrt() as u64)
    // };
    // let max_op_time = op_times
    //     .iter()
    //     .last()
    //     .copied()
    //     .unwrap_or(NanosDurationU64::from_ticks(0));
    // let median_op_time = median(&op_times, NanosDurationU64::from_ticks(0));
    // let time = timings.total_time(overall_stats);
    let stats: Vec<NanosDurationU64> = op_stats.iter().map(|s| timings.total_time(s)).collect();
    let data_stats: RunData = stats.as_slice().into();
    println!(
        "{:>22} - median: {:>5} µs, avg per op: {:>6} µs, std: {:>6} µs, max: {:>8} µs",
        name,
        // time.to_micros() / overall_stats.total_operations as u64,
        data_stats.median.to_micros(),
        data_stats.avg.to_micros(),
        data_stats.std_dev.to_micros(),
        data_stats.max.to_micros(),
    );
    data_stats
}

impl NorFlashApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let tree = egui_tiles::Tree::new_tabs("plot_tabs", vec![]);
        println!("Created root tile: {:?}", tree.root);
        let app_state = Arc::new(Mutex::new(AppState::default()));
        let mut runner = Runner::new();
        runner.run(Arc::clone(&app_state));

        Self {
            tree,
            plot_tabs: Default::default(),
            runner,
            state: app_state,
        }
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
        let (snapshot, new_snapshot) = {
            // take a clone of current state
            let mut guard = self.state.lock().unwrap();
            if let Some(new_snapshot) = guard.new_snapshot.take() {
                guard.snapshot = Some(new_snapshot.clone());
                (Some(new_snapshot.clone()), Some(new_snapshot))
            } else {
                (guard.snapshot.clone(), None)
            }
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
                if ui.button("Run map store benchmark").clicked() {
                    self.runner.start_workload(Workload::SqBenchmarkMapStore);
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

            let mut tabs = vec![];
            if let Some(new_snapshot) = &new_snapshot {
                // if !self.plot_tabs.contains_key("page_erase_grid") {
                let tab = self
                    .tree
                    .tiles
                    .insert_pane(TabPane::PageEraseGrid(new_snapshot.clone()));
                self.plot_tabs.insert("page_erase_grid".to_string(), tab);
                tabs.push(tab);
                // }
            }
            // draw_page_grid(ui, snapshot);

            let app_state = &self.state.lock().unwrap();
            // Render each plots in a tabs
            for (name, data_stats) in app_state.run_data.iter() {
                // Add entries that were not present
                if !self.plot_tabs.contains_key(name) {
                    for &plot_type in &[
                        PlotType::Line,
                        PlotType::Histogram,
                        // PlotType::TotalTimeHistogram,
                    ] {
                        let tab = self.tree.tiles.insert_pane(TabPane::Plot(PlotTabPane {
                            title: format!("{} ({})", name, plot_type),
                            plot_data: data_stats.clone(),
                            plot_type,
                        }));
                        self.plot_tabs.insert(name.clone(), tab);
                        tabs.push(tab);
                    }
                }
            }
            for tab in tabs {
                if let Some(root) = self.tree.root
                    && let Some(tile) = self.tree.tiles.get_mut(root)
                    && let egui_tiles::Tile::Container(container) = tile
                {
                    container.add_child(tab);
                } else {
                    self.tree.root = Some(self.tree.tiles.insert_tab_tile(vec![tab]));
                }
            }

            let mut behavior = TabBehavior {};
            self.tree.ui(&mut behavior, ui);
            // } else {
            //     ui.label("Waiting for first snapshot...");
            // }
        });

        ctx.request_repaint(); // continuous repaint to show new snapshots
    }
}

fn draw_page_grid(ui: &mut egui::Ui, snapshot: &FlashSnapshot) -> InnerResponse<()> {
    let page_cycles = &snapshot.page_cycles;
    // if page_cycles.is_empty() {
    //     ui.label("No pages");
    //     return;
    // }

    // For now, lay out as a simple square-ish grid.
    let pages = page_cycles.len();
    let cols = (pages as f32).sqrt().ceil() as usize;
    let rows = (pages + cols - 1) / cols;

    let cell_size = egui::vec2(35.0, 35.0);
    let red_cycle_count = page_cycles.iter().copied().max().unwrap_or(100).max(20); // cycles at which cell is fully red
    let font = FontId::proportional(11.0);

    egui::Grid::new("page_grid")
        .min_col_width(cell_size.x) // override default ~40
        .min_row_height(cell_size.y) // override default interact_size.y
        .max_col_width(cell_size.x) // clamp to exactly 30
        .spacing(egui::vec2(1.0, 1.0)) // 1px-ish spacing between cells
        .show(ui, |ui| {
            ui.set_width(cell_size.x * cols as f32);
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
                                ui.style_mut().override_font_id = Some(font.clone());
                                Label::new(format!("{}", cycles)).center(ui);
                                // ui.set_min_size(cell_size);
                                // Force the content area to 30×30
                                // ui.allocate_exact_size(cell_size, egui::Sense::hover());
                                ui.take_available_space();
                            });

                        idx += 1;
                    }
                }
                ui.end_row();
            }
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlotType {
    Line,
    Histogram,
    TotalTimeHistogram,
}
impl Display for PlotType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlotType::Line => write!(f, "Line"),
            PlotType::Histogram => write!(f, "Histogram"),
            PlotType::TotalTimeHistogram => write!(f, "Time Histogram"),
        }
    }
}

#[derive(Debug)]
struct PlotTabPane {
    title: String,
    plot_data: RunData,
    plot_type: PlotType,
}
#[derive(Debug)]
enum TabPane {
    Label(String),
    Plot(PlotTabPane),
    PageEraseGrid(FlashSnapshot),
}
impl TabPane {
    fn title(&self) -> &str {
        match self {
            TabPane::Label(t) => t,
            TabPane::Plot(p) => &p.title,
            TabPane::PageEraseGrid(_) => "Page erase cycles",
        }
    }
}

pub fn format_micros(micros: f64) -> String {
    if micros < 0.0 {
        return "".to_string();
    } else if micros == 0.0 {
        return "0 s".to_string();
    } else if micros >= 2_000_000.0 {
        format!("{:.0} s", micros / 1_000_000.0)
    } else if micros >= 2_000.0 {
        format!("{:.0} ms", micros / 1_000.0)
    } else {
        format!("{:.0} µs", micros)
    }
}

struct TabBehavior {}

impl egui_tiles::Behavior<TabPane> for TabBehavior {
    fn tab_title_for_pane(&mut self, pane: &TabPane) -> egui::WidgetText {
        pane.title().into()
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut TabPane,
    ) -> egui_tiles::UiResponse {
        match pane {
            TabPane::Label(text) => {
                ui.label(text.as_str());
            }
            TabPane::PageEraseGrid(snapshot) => {
                draw_page_grid(ui, snapshot);
            }
            TabPane::Plot(pane) => {
                let plot_data = &pane.plot_data;
                if pane.plot_type != PlotType::Line {
                    let max = plot_data.max.to_micros();
                    let steps = 100;
                    let step_size = (max / steps).max(1);
                    let micros = plot_data
                        .data
                        .iter()
                        .map(|t| t.to_micros())
                        .collect::<Vec<_>>();

                    let get_step_count = |st| {
                        micros
                            .iter()
                            .filter(|&&t| t >= st && t < st + step_size)
                            .count() as f64
                            / micros.len() as f64
                    };
                    let count_chart = BarChart::new(
                        "Occurrence Distribution",
                        (0..max + step_size)
                            .step_by(step_size as usize)
                            .map(|t| (t as f64, get_step_count(t)))
                            .map(|(x, n)| Bar::new(x, n).width(step_size as f64 / 2.0))
                            .collect(),
                    )
                    .element_formatter(Box::new(|bar, _chart| format!("{:.2}%", bar.value * 100.0)))
                    .color(egui::Color32::BLUE);

                    let total_time = micros.iter().sum::<u64>();
                    let get_step_sum = |st| {
                        micros
                            .iter()
                            .filter(|&&t| t >= st && t < st + step_size)
                            .sum::<u64>() as f64
                            / total_time as f64
                    };
                    let time_chart = BarChart::new(
                        "Time Distribution",
                        (0..max + step_size)
                            .step_by(step_size as usize)
                            .map(|t| (t as f64, get_step_sum(t)))
                            .map(|(x, n)| {
                                Bar::new(x + step_size as f64 / 2.0, n)
                                    .width(step_size as f64 / 2.0)
                            })
                            .collect(),
                    )
                    .element_formatter(Box::new(|bar, _chart| format!("{:.2}%", bar.value * 100.0)))
                    .color(egui::Color32::GREEN);

                    Plot::new(format!("histogram_{}", pane.title))
                        .legend(Legend::default())
                        .coordinates_formatter(
                            Corner::LeftTop,
                            CoordinatesFormatter::new(|value, _bounds| {
                                format!("{:.2}%", value.y * 100.0)
                            }),
                        )
                        .custom_y_axes(vec![AxisHints::new_y().formatter(|grid_mark, _range| {
                            format!("{:.2}%", grid_mark.value * 100.0)
                        })])
                        .custom_x_axes(vec![
                            AxisHints::new_x()
                                .formatter(|grid_mark, _range| format_micros(grid_mark.value)),
                            AxisHints::new_x()
                                .placement(HPlacement::Right)
                                .formatter(|grid_mark, _range| format_micros(grid_mark.value)),
                        ])
                        .label_formatter(|_name, value| format!("{:.2}%", value.y * 100.0))
                        .clamp_grid(true)
                        .show(ui, |ui| {
                            ui.bar_chart(count_chart);
                            ui.bar_chart(time_chart);
                        });
                    return egui_tiles::UiResponse::None;
                } else {
                    let points: PlotPoints = plot_data
                        .data
                        .iter()
                        .enumerate()
                        .map(|(i, &y)| [i as f64, y.to_micros() as f64])
                        .collect();
                    let line = Line::new(pane.title.clone(), points).name("data");
                    let median_line = HLine::new("median", plot_data.median.to_micros() as f64)
                        .style(egui_plot::LineStyle::Dashed { length: 2.0 });
                    let avg_line = HLine::new("avg", plot_data.avg.to_micros() as f64)
                        .style(egui_plot::LineStyle::Dotted { spacing: 2.0 });
                    Plot::new(format!("plot_{}", pane.title))
                        .legend(Default::default())
                        .coordinates_formatter(
                            Corner::LeftTop,
                            CoordinatesFormatter::new(|value, _| format_micros(value.y)),
                        )
                        .custom_y_axes(vec![
                            AxisHints::new_y()
                                .label("time")
                                .formatter(|grid_mark, _range| format_micros(grid_mark.value)),
                        ])
                        .label_formatter(|name, value| {
                            if !name.is_empty() {
                                format!("{}: {:.0}", name, format_micros(value.y))
                            } else {
                                "".to_owned()
                            }
                        })
                        .clamp_grid(true)
                        .show(ui, |ui| {
                            ui.hline(median_line);
                            ui.hline(avg_line);
                            ui.line(line);
                        });
                }
            }
        }
        egui_tiles::UiResponse::None
    }
}
