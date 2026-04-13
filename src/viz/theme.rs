//! Central palette for the dashboard (flat fills, no expensive effects).

use eframe::egui;

pub const BG_WINDOW: egui::Color32 = egui::Color32::from_rgb(12, 12, 16);
pub const BG_PANEL: egui::Color32 = egui::Color32::from_rgb(16, 16, 22);
pub const BG_EXTREME: egui::Color32 = egui::Color32::from_rgb(8, 8, 11);
pub const BG_WIDGET: egui::Color32 = egui::Color32::from_rgb(24, 24, 30);
pub const BG_CARD: egui::Color32 = egui::Color32::from_rgb(22, 22, 30);
pub const STROKE_CARD: egui::Color32 = egui::Color32::from_rgb(38, 42, 56);
pub const STROKE_TOP: egui::Color32 = egui::Color32::from_rgb(40, 44, 58);
pub const FG_TOP_BAR: egui::Color32 = egui::Color32::from_rgb(20, 20, 28);
pub const ACCENT_TITLE: egui::Color32 = egui::Color32::from_rgb(130, 175, 255);
pub const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(140, 140, 155);
pub const TEXT_CARD_TITLE: egui::Color32 = egui::Color32::from_rgb(210, 215, 235);
pub const TEXT_FG: egui::Color32 = egui::Color32::from_rgb(200, 200, 215);

pub const PLOT_BID: egui::Color32 = egui::Color32::from_rgb(80, 220, 120);
pub const PLOT_ASK: egui::Color32 = egui::Color32::from_rgb(255, 95, 95);
pub const PLOT_LATENCY: egui::Color32 = egui::Color32::from_rgb(170, 130, 255);
pub const PLOT_IMBALANCE: egui::Color32 = egui::Color32::from_rgb(255, 190, 85);

#[inline]
pub fn plot_spread_fill() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(60, 140, 220, 45)
}

#[inline]
pub fn plot_ref_line() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(160, 160, 180, 120)
}

pub const SELECTION: egui::Color32 = egui::Color32::from_rgb(72, 98, 160);
pub const INACTIVE: egui::Color32 = egui::Color32::from_rgb(34, 36, 48);
pub const HOVERED: egui::Color32 = egui::Color32::from_rgb(48, 52, 68);
pub const ACTIVE: egui::Color32 = egui::Color32::from_rgb(56, 62, 88);
pub const OPEN: egui::Color32 = egui::Color32::from_rgb(40, 44, 58);
