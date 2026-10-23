//! Glance: screen OCR and translation from the Windows system tray.

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod app;
mod capture;
mod config;
mod error;
mod hotkey;
mod http;
mod imaging;
mod languages;
mod links;
mod llm;
mod models;
mod ocr;
mod overlay;
mod popup;
mod providers;
mod secrets;
mod settings;
mod strings;
mod translate;
mod tray;
mod win32;
mod window_role;

slint::include_modules!();

fn main() {
    let Some(_instance) = win32::SingleInstance::acquire() else {
        win32::message_box(strings::ALREADY_RUNNING);
        return;
    };
    win32::allow_dark_menus();
    if let Err(e) = app::run() {
        win32::message_box(&strings::startup_failed(&e.to_string()));
    }
}
