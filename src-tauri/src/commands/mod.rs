//! Команды Tauri, разнесённые по предметным областям.
//!
//! Регистрация всех команд - в `crate::run`. Состояние приложения (`AppState`) живёт в `lib.rs`:
//! команды получают его через `State` и сами ничего глобального не заводят.

pub mod app;
pub mod db;
pub mod desktop;
pub mod docker;
pub mod files;
pub mod fleet;
pub mod host;
pub mod observability;
pub mod profile;
pub mod session;
pub mod tasks;
pub mod tools;
