//! Native egui dashboard (feature `dashboard`).

use crate::viz::store::{SymbolSnapshot, VizStore, HEATMAP_PRICE_ROWS, HEATMAP_TIME_COLS};
use eframe::egui;
use egui_plot::{Line, LineStyle, Plot, PlotPoints, Polygon};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct DashboardApp {
    symbols: Vec<String>,
    viz: Arc<VizStore>,
    selected_id: u16,
    shutdown: Arc<AtomicBool>,
}

impl DashboardApp {
    pub fn new(symbols: Vec<String>, viz: Arc<VizStore>, shutdown: Arc<AtomicBool>) -> Self {
        let selected_id = 0u16;
        viz.heatmap_focus_symbol.store(selected_id, Ordering::Relaxed);
        Self {
            symbols,
            viz,
            selected_id,
            shutdown,
        }
    }
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) {
            self.shutdown.store(true, Ordering::SeqCst);
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Symbol:");
                let mut sel = self.selected_id as usize;
                egui::ComboBox::from_id_salt("sym")
                    .selected_text(
                        self.symbols
                            .get(sel)
                            .map(|s| s.as_str())
                            .unwrap_or("—"),
                    )
                    .show_ui(ui, |ui| {
                        for (i, name) in self.symbols.iter().enumerate() {
                            if ui.selectable_value(&mut sel, i, name).clicked() {}
                        }
                    });
                if sel < self.symbols.len() {
                    let new_id = sel as u16;
                    if new_id != self.selected_id {
                        self.selected_id = new_id;
                        self.viz.heatmap_focus_symbol.store(new_id, Ordering::Relaxed);
                    }
                }
            });
            ui.label(
                "NBBO heatmap updates for the selected symbol only. Latency = local wall clock − Alpaca quote time (skew possible).",
            );
        });

        let snap = self.viz.snapshot_symbol(self.selected_id);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    spread_plot(ui, &snap);
                    heatmap_view(ui, ctx, &snap);
                });
                ui.vertical(|ui| {
                    imbalance_plot(ui, &snap);
                    latency_plot(ui, &snap);
                });
            });
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

fn spread_plot(ui: &mut egui::Ui, snap: &SymbolSnapshot) {
    ui.label("Bid–ask (shaded spread)");
    Plot::new("spread")
        .height(200.0)
        .allow_zoom(false)
        .allow_scroll(false)
        .show(ui, |plot_ui| {
            if snap.samples.len() < 2 {
                return;
            }
            let mut hull: Vec<[f64; 2]> = snap.samples.iter().map(|s| [s.exch_ts_sec, s.bid]).collect();
            for s in snap.samples.iter().rev() {
                hull.push([s.exch_ts_sec, s.ask]);
            }
            plot_ui.polygon(
                Polygon::new(PlotPoints::from_iter(hull.iter().copied()))
                    .fill_color(egui::Color32::from_rgba_unmultiplied(80, 120, 200, 60))
                    .stroke(egui::Stroke::new(0.0, egui::Color32::TRANSPARENT)),
            );
            let bid_pts: PlotPoints = snap.samples.iter().map(|s| [s.exch_ts_sec, s.bid]).collect();
            let ask_pts: PlotPoints = snap.samples.iter().map(|s| [s.exch_ts_sec, s.ask]).collect();
            plot_ui.line(
                Line::new(bid_pts)
                    .name("bid")
                    .color(egui::Color32::from_rgb(50, 180, 80))
                    .width(1.5),
            );
            plot_ui.line(
                Line::new(ask_pts)
                    .name("ask")
                    .color(egui::Color32::from_rgb(200, 60, 60))
                    .width(1.5),
            );
        });
}

fn heatmap_view(ui: &mut egui::Ui, ctx: &egui::Context, snap: &SymbolSnapshot) {
    ui.label("NBBO liquidity (log-scaled size at bid/ask vs time)");
    let expected = HEATMAP_PRICE_ROWS * HEATMAP_TIME_COLS;
    if snap.heatmap.len() != expected {
        ui.label("…");
        return;
    }
    let mut img = egui::ColorImage::new([HEATMAP_TIME_COLS, HEATMAP_PRICE_ROWS], egui::Color32::BLACK);
    let mx = snap
        .heatmap
        .iter()
        .cloned()
        .fold(0f32, f32::max)
        .max(1e-9);
    for row in 0..HEATMAP_PRICE_ROWS {
        for col in 0..HEATMAP_TIME_COLS {
            let v = snap.heatmap[row * HEATMAP_TIME_COLS + col] / mx;
            let t = (v.clamp(0.0, 1.0) * 255.0) as u8;
            let (r, g, b) = heat_color(t);
            img[(col, HEATMAP_PRICE_ROWS - 1 - row)] = egui::Color32::from_rgb(r, g, b);
        }
    }
    let tex = ctx.load_texture("nbbo_heat", img, egui::TextureOptions::LINEAR);
    ui.add(
        egui::Image::new(egui::load::SizedTexture::new(tex.id(), tex.size_vec2()))
            .max_width(360.0),
    );
}

#[inline]
fn heat_color(t: u8) -> (u8, u8, u8) {
    // cool (low) → warm (high)
    if t < 128 {
        let u = t * 2;
        (0, u, 255u8.saturating_sub(u))
    } else {
        let u = (t - 128) * 2;
        (u, 255u8.saturating_sub(u), 0)
    }
}

fn imbalance_plot(ui: &mut egui::Ui, snap: &SymbolSnapshot) {
    ui.label(format!(
        "Trade imbalance (Lee–Ready, cum Δ = {})",
        snap.cum_delta
    ));
    Plot::new("imbalance")
        .height(200.0)
        .allow_zoom(false)
        .allow_scroll(false)
        .show(ui, |plot_ui| {
            if snap.delta_series.is_empty() {
                return;
            }
            let pts: PlotPoints = snap
                .delta_series
                .iter()
                .map(|(t, d)| [*t, *d as f64])
                .collect();
            plot_ui.line(
                Line::new(pts)
                    .name("cum Δ")
                    .color(egui::Color32::from_rgb(200, 140, 40))
                    .style(LineStyle::Solid)
                    .width(1.5),
            );
        });
}

fn latency_plot(ui: &mut egui::Ui, snap: &SymbolSnapshot) {
    ui.label("Latency (ms): local_rx − exchange_ts");
    Plot::new("latency")
        .height(200.0)
        .allow_zoom(false)
        .allow_scroll(false)
        .include_y(0.0)
        .show(ui, |plot_ui| {
            if snap.samples.is_empty() {
                return;
            }
            let pts: PlotPoints = snap
                .samples
                .iter()
                .map(|s| {
                    let y = s.latency_ms.clamp(-500.0, 500.0) as f64;
                    [s.exch_ts_sec, y]
                })
                .collect();
            plot_ui.line(
                Line::new(pts)
                    .name("ms")
                    .color(egui::Color32::from_rgb(120, 80, 200))
                    .width(1.5),
            );
        });
}
