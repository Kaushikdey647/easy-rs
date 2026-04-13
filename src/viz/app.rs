//! Native egui dashboard (feature `dashboard`).

use crate::viz::pipeline::{
    DashboardPipeline, SymbolRender, HEATMAP_PRICE_ROWS, HEATMAP_TIME_COLS, NBUCKETS,
};
use crate::viz::theme;
use eframe::egui;
use egui_plot::{GridMark, Line, LineStyle, Plot, PlotPoints, Polygon};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub fn dashboard_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals.dark_mode = true;
    style.visuals.window_fill = theme::BG_WINDOW;
    style.visuals.panel_fill = theme::BG_PANEL;
    style.visuals.extreme_bg_color = theme::BG_EXTREME;
    style.visuals.widgets.noninteractive.bg_fill = theme::BG_WIDGET;
    style.visuals.widgets.noninteractive.fg_stroke.color = theme::TEXT_FG;
    style.visuals.widgets.inactive.bg_fill = theme::INACTIVE;
    style.visuals.widgets.hovered.bg_fill = theme::HOVERED;
    style.visuals.widgets.active.bg_fill = theme::ACTIVE;
    style.visuals.selection.bg_fill = theme::SELECTION;
    style.visuals.widgets.open.bg_fill = theme::OPEN;
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(10.0);
    ctx.set_style(style);
}

struct PlotScratch {
    bid: Vec<[f64; 2]>,
    ask: Vec<[f64; 2]>,
    lat: Vec<[f64; 2]>,
    imb: Vec<[f64; 2]>,
}

impl PlotScratch {
    fn new() -> Self {
        Self {
            bid: Vec::with_capacity(NBUCKETS),
            ask: Vec::with_capacity(NBUCKETS),
            lat: Vec::with_capacity(NBUCKETS),
            imb: Vec::with_capacity(NBUCKETS),
        }
    }
}

pub struct DashboardApp {
    symbols: Vec<String>,
    pipeline: Arc<DashboardPipeline>,
    selected_id: u16,
    shutdown: Arc<AtomicBool>,
    scratch: PlotScratch,
    last_heatmap_version: u64,
    last_heatmap_focus_id: u16,
    heatmap_tex: Option<egui::TextureHandle>,
    empty_symbol: SymbolRender,
}

impl DashboardApp {
    pub fn new(symbols: Vec<String>, pipeline: Arc<DashboardPipeline>, shutdown: Arc<AtomicBool>) -> Self {
        let selected_id = 0u16;
        pipeline
            .heatmap_focus_symbol
            .store(selected_id, Ordering::Relaxed);
        Self {
            symbols,
            pipeline,
            selected_id,
            shutdown,
            scratch: PlotScratch::new(),
            last_heatmap_version: 0,
            last_heatmap_focus_id: u16::MAX,
            heatmap_tex: None,
            empty_symbol: SymbolRender::default(),
        }
    }
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) {
            self.shutdown.store(true, Ordering::SeqCst);
        }

        let snap = self.pipeline.snapshot.load_full();
        let sym: &SymbolRender = snap
            .symbols
            .get(self.selected_id as usize)
            .unwrap_or(&self.empty_symbol);

        if self.last_heatmap_focus_id != self.selected_id {
            self.last_heatmap_focus_id = self.selected_id;
            self.last_heatmap_version = u64::MAX;
            self.heatmap_tex = None;
        }

        egui::TopBottomPanel::top("top")
            .frame(
                egui::Frame::none()
                    .fill(theme::FG_TOP_BAR)
                    .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                    .stroke(egui::Stroke::new(1.0, theme::STROKE_TOP)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("easy-rs")
                            .strong()
                            .size(16.0)
                            .color(theme::ACCENT_TITLE),
                    );
                    ui.separator();
                    ui.label("Symbol");
                    let mut sel = self.selected_id as usize;
                    egui::ComboBox::from_id_salt("sym")
                        .selected_text(
                            self.symbols
                                .get(sel)
                                .map(|s| s.as_str())
                                .unwrap_or("—"),
                        )
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            for (i, name) in self.symbols.iter().enumerate() {
                                ui.selectable_value(&mut sel, i, name);
                            }
                        });
                    if sel < self.symbols.len() {
                        let new_id = sel as u16;
                        if new_id != self.selected_id {
                            self.selected_id = new_id;
                            self.pipeline
                                .heatmap_focus_symbol
                                .store(new_id, Ordering::Relaxed);
                        }
                    }
                    let dropped = self.pipeline.feed_dropped.load(Ordering::Relaxed);
                    if dropped > 0 {
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!("feed drops (queue full): {dropped}"))
                                .small()
                                .color(egui::Color32::from_rgb(255, 160, 100)),
                        );
                    }
                });
                ui.label(
                    egui::RichText::new(
                        "NBBO from Alpaca · heatmap follows selected symbol · latency = local wall − exchange quote time · pipeline uses monotonic ingest stamp",
                    )
                    .small()
                    .color(theme::TEXT_MUTED),
                );
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::BG_WINDOW))
            .show(ctx, |ui| {
                let gap = 10.0;
                let full = ui.available_size();
                let col_w = (full.x - gap) * 0.5;
                let row_h = (full.y - gap) * 0.5;

                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        chart_card(ui, col_w, row_h, "Bid–ask spread", |ui| {
                            spread_plot(ui, row_h - 36.0, &sym, &mut self.scratch);
                        });
                        chart_card(ui, col_w, row_h, "Quote latency", |ui| {
                            latency_plot(ui, row_h - 36.0, &sym, &mut self.scratch);
                        });
                    });
                    ui.add_space(gap);
                    ui.horizontal(|ui| {
                        chart_card(ui, col_w, row_h, "NBBO liquidity heatmap", |ui| {
                            heatmap_view(
                                ui,
                                ctx,
                                col_w,
                                row_h - 36.0,
                                &sym,
                                snap.heatmap_version,
                                &mut self.last_heatmap_version,
                                &mut self.heatmap_tex,
                            );
                        });
                        let imb_title = format!("Cumulative trade imbalance · Δ = {}", sym.cum_delta);
                        chart_card(ui, col_w, row_h, &imb_title, |ui| {
                            imbalance_plot(ui, row_h - 36.0, &sym, &mut self.scratch);
                        });
                    });
                });
            });

        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

fn chart_card(ui: &mut egui::Ui, width: f32, height: f32, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(theme::BG_CARD)
        .rounding(egui::Rounding::same(10.0))
        .stroke(egui::Stroke::new(1.0, theme::STROKE_CARD))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_width(width);
            ui.set_min_height(height);
            ui.label(
                egui::RichText::new(title)
                    .strong()
                    .size(13.0)
                    .color(theme::TEXT_CARD_TITLE),
            );
            ui.add_space(6.0);
            add_contents(ui);
        });
}

fn x_fmt_secs() -> impl Fn(GridMark, &std::ops::RangeInclusive<f64>) -> String + Clone {
    |mark, _range| format!("{:.1}s", mark.value)
}

#[inline]
fn sanitize_nbbo(bid: f64, ask: f64) -> Option<(f64, f64)> {
    let mut b = bid;
    let mut a = ask;
    if !b.is_finite() || !a.is_finite() {
        return None;
    }
    if b > a {
        std::mem::swap(&mut b, &mut a);
    }
    Some((b, a))
}

fn fill_xy_i64(t: &[f64], y: &[i64], x_min: f64, x_max: f64, out: &mut Vec<[f64; 2]>) {
    out.clear();
    let n = t.len().min(y.len());
    for i in 0..n {
        let x = t[i];
        if x >= x_min && x <= x_max {
            out.push([x, y[i] as f64]);
        }
    }
}

/// EMA on the Y axis for the **already viewport-filtered** points (display only).
fn ema_y(points: &[[f64; 2]], alpha: f64) -> Vec<[f64; 2]> {
    if points.is_empty() {
        return vec![];
    }
    let mut out = Vec::with_capacity(points.len());
    let mut ey = points[0][1];
    for p in points {
        ey = alpha * p[1] + (1.0 - alpha) * ey;
        out.push([p[0], ey]);
    }
    out
}

fn spread_plot(ui: &mut egui::Ui, plot_h: f32, sym: &SymbolRender, scratch: &mut PlotScratch) {
    if sym.t_rel.len() < 2 {
        ui.label(
            egui::RichText::new("Waiting for quotes…")
                .small()
                .color(theme::TEXT_MUTED),
        );
    }
    Plot::new("spread")
        .height(plot_h.max(120.0))
        .allow_zoom(true)
        .allow_drag(true)
        .allow_scroll(false)
        .show_grid(true)
        .show_axes(true)
        .x_axis_formatter(x_fmt_secs())
        .legend(egui_plot::Legend::default().position(egui_plot::Corner::RightTop))
        .show(ui, |plot_ui| {
            if sym.t_rel.len() < 2 {
                return;
            }
            let bounds = plot_ui.plot_bounds();
            let x_min = bounds.min()[0];
            let x_max = bounds.max()[0];

            scratch.bid.clear();
            scratch.ask.clear();
            let n = sym.t_rel.len().min(sym.bid.len()).min(sym.ask.len());
            for i in 0..n {
                let x = sym.t_rel[i];
                if x < x_min || x > x_max {
                    continue;
                }
                let (b, a) = match sanitize_nbbo(sym.bid[i], sym.ask[i]) {
                    Some(p) => p,
                    None => continue,
                };
                scratch.bid.push([x, b]);
                scratch.ask.push([x, a]);
            }

            if scratch.bid.len() < 2 {
                return;
            }

            let bid_s = ema_y(&scratch.bid, 0.22);
            let ask_s = ema_y(&scratch.ask, 0.22);

            let mut hull: Vec<[f64; 2]> = bid_s.iter().copied().collect();
            for p in ask_s.iter().rev() {
                hull.push(*p);
            }
            if hull.len() >= 3 {
                plot_ui.polygon(
                    Polygon::new(PlotPoints::from_iter(hull.iter().copied()))
                        .fill_color(theme::plot_spread_fill())
                        .stroke(egui::Stroke::new(0.0, egui::Color32::TRANSPARENT)),
                );
            }

            plot_ui.line(
                Line::new(PlotPoints::from_iter(bid_s.iter().copied()))
                    .name("Bid (smooth)")
                    .color(theme::PLOT_BID)
                    .width(2.0),
            );
            plot_ui.line(
                Line::new(PlotPoints::from_iter(ask_s.iter().copied()))
                    .name("Ask (smooth)")
                    .color(theme::PLOT_ASK)
                    .width(2.0),
            );
        });
}

fn heatmap_view(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    max_w: f32,
    max_h: f32,
    sym: &SymbolRender,
    heatmap_version: u64,
    last_ver: &mut u64,
    tex: &mut Option<egui::TextureHandle>,
) {
    let expected = HEATMAP_PRICE_ROWS * HEATMAP_TIME_COLS;
    if sym.heatmap.len() != expected {
        ui.label(
            egui::RichText::new("Heatmap for selected symbol…")
                .color(egui::Color32::GRAY),
        );
        return;
    }

    if heatmap_version != *last_ver || tex.is_none() {
        let mut img = egui::ColorImage::new([HEATMAP_TIME_COLS, HEATMAP_PRICE_ROWS], egui::Color32::BLACK);
        let mx = sym
            .heatmap
            .iter()
            .cloned()
            .fold(0f32, f32::max)
            .max(1e-9);
        for row in 0..HEATMAP_PRICE_ROWS {
            for col in 0..HEATMAP_TIME_COLS {
                let v = sym.heatmap[row * HEATMAP_TIME_COLS + col] / mx;
                let t = (v.clamp(0.0, 1.0) * 255.0) as u8;
                let (r, g, b) = heat_color(t);
                img[(col, HEATMAP_PRICE_ROWS - 1 - row)] = egui::Color32::from_rgb(r, g, b);
            }
        }
        *tex = Some(ctx.load_texture("nbbo_heat", img, egui::TextureOptions::LINEAR));
        *last_ver = heatmap_version;
    }

    let Some(t) = tex.as_ref() else {
        return;
    };
    let tw = max_w - 8.0;
    let th = max_h.max(80.0);
    let iw = HEATMAP_TIME_COLS as f32;
    let ih = HEATMAP_PRICE_ROWS as f32;
    let scale = (tw / iw).min(th / ih);
    let size = egui::vec2(iw * scale, ih * scale);
    ui.add(
        egui::Image::new(egui::load::SizedTexture::new(t.id(), t.size_vec2())).max_size(size),
    );
}

#[inline]
fn heat_color(t: u8) -> (u8, u8, u8) {
    if t < 128 {
        let u = t * 2;
        (20, u, 255u8.saturating_sub(u))
    } else {
        let u = (t - 128) * 2;
        (u, 255u8.saturating_sub(u), 40)
    }
}

fn imbalance_plot(ui: &mut egui::Ui, plot_h: f32, sym: &SymbolRender, scratch: &mut PlotScratch) {
    if sym.delta_t_rel.is_empty() {
        ui.label(
            egui::RichText::new("No classified trades yet (needs live trades on your feed).")
                .small()
                .color(theme::TEXT_MUTED),
        );
    }
    Plot::new("imbalance")
        .height(plot_h.max(120.0))
        .allow_zoom(true)
        .allow_drag(true)
        .allow_scroll(false)
        .show_grid(true)
        .show_axes(true)
        .x_axis_formatter(x_fmt_secs())
        .legend(egui_plot::Legend::default().position(egui_plot::Corner::RightTop))
        .show(ui, |plot_ui| {
            if sym.delta_t_rel.is_empty() {
                return;
            }
            let bounds = plot_ui.plot_bounds();
            let x_min = bounds.min()[0];
            let x_max = bounds.max()[0];
            fill_xy_i64(&sym.delta_t_rel, &sym.delta_cum, x_min, x_max, &mut scratch.imb);
            if scratch.imb.is_empty() {
                return;
            }
            let sm = ema_y(&scratch.imb, 0.35);
            plot_ui.line(
                Line::new(PlotPoints::from_iter(sm.iter().copied()))
                    .name("Cum Δ (smooth)")
                    .color(theme::PLOT_IMBALANCE)
                    .style(LineStyle::Solid)
                    .width(2.0),
            );
        });
}

fn latency_plot(ui: &mut egui::Ui, plot_h: f32, sym: &SymbolRender, scratch: &mut PlotScratch) {
    if sym.t_rel.is_empty() {
        ui.label(
            egui::RichText::new("Waiting for quotes…")
                .small()
                .color(theme::TEXT_MUTED),
        );
    }
    Plot::new("latency")
        .height(plot_h.max(120.0))
        .allow_zoom(true)
        .allow_drag(true)
        .allow_scroll(false)
        .show_grid(true)
        .show_axes(true)
        .x_axis_formatter(x_fmt_secs())
        .legend(egui_plot::Legend::default().position(egui_plot::Corner::RightTop))
        .show(ui, |plot_ui| {
            if sym.t_rel.is_empty() {
                return;
            }
            let bounds = plot_ui.plot_bounds();
            let x_min = bounds.min()[0];
            let x_max = bounds.max()[0];
            scratch.lat.clear();
            let n = sym.t_rel.len().min(sym.lat_ms.len());
            for i in 0..n {
                let x = sym.t_rel[i];
                if x < x_min || x > x_max {
                    continue;
                }
                let y = sym.lat_ms[i].clamp(-250.0, 250.0) as f64;
                scratch.lat.push([x, y]);
            }
            if scratch.lat.is_empty() {
                return;
            }
            let sm = ema_y(&scratch.lat, 0.28);
            plot_ui.line(
                Line::new(PlotPoints::from_iter(sm.iter().copied()))
                    .name("Latency ms (smooth)")
                    .color(theme::PLOT_LATENCY)
                    .width(2.0),
            );
            let x0 = scratch.lat.first().map(|p| p[0]).unwrap_or(0.0);
            let x1 = scratch.lat.last().map(|p| p[0]).unwrap_or(x0);
            plot_ui.line(
                Line::new(PlotPoints::from_iter([[x0, 0.0], [x1, 0.0]]))
                    .name("0 ms")
                    .color(theme::plot_ref_line())
                    .width(1.0)
                    .style(LineStyle::dotted_loose()),
            );
        });
}
