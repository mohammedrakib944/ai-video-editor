// Hide the console window in release builds; `--mcp-stdio` still works over inherited pipes.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]



fn main() {
    if std::env::args().any(|a| a == "--mcp-stdio") {
        if let Err(e) = ai_video_editor::mcp::run_stdio_bridge() {
            eprintln!("mcp bridge error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(e) = ai_video_editor::ui::run() {
        eprintln!("failed to start: {e}");
        std::process::exit(1);
    }
}
