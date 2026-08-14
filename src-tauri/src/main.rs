// Binary entry point: calls the library entry run()
// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    revue_gate_lib::run()
}
