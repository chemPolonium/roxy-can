use std::io::IsTerminal;

mod aggregate;
mod app;
mod block;
mod bus;
mod can;
mod can_arxml;
mod channel;
mod cli;
mod config;
mod core_loop;
mod dbc;
mod decode;
mod derived;
mod eth;
mod export;
mod fr_db;
mod generator;
mod hw;
mod load;
mod log;
// The unified model is frozen in Phase 1; the Phase 2 importer and the Phase
// 4 runtime switch are its first consumers.
#[allow(dead_code)]
mod model;
mod node;
mod observe;
mod profile;
mod project;
mod recorder;
mod reference;
mod replay_map;
mod sample_data;
mod script;
mod sim;
mod source;
mod spec;
mod state_seg;
mod trace;
mod trigger;
mod web;
mod workspace;

/// 主线阶段 2 验收：不开 UI 跑完整仿真 + 录制 + 导出。
#[cfg(test)]
#[path = "headless_tests.rs"]
mod headless_tests;

#[cfg(test)]
mod can_arxml_tests;

#[cfg(test)]
mod test_assets;
#[cfg(test)]
mod test_assets_data;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        let open = std::env::var_os("ROXY_WEB_NO_OPEN").is_none();
        run_web(&args, open);
        return;
    }
    if args.iter().any(|arg| arg == "--web") {
        run_web(&args, false);
        return;
    }
    match cli::parse_args(&args) {
        Ok(cli::Cli::Gui) => println!("{}", cli::usage()),
        Ok(cli::Cli::Help(text)) => println!("{text}"),
        Ok(cli::Cli::Run(opts)) => match cli::run(&opts) {
            Ok(report) => println!("{report}"),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        Ok(cli::Cli::CheckScripts(check)) => match cli::check_scripts(&check) {
            Ok(report) => println!("{report}"),
            Err(report) => {
                eprintln!("{report}");
                std::process::exit(2);
            }
        },
        Ok(cli::Cli::Convert(input, output)) => match cli::convert_log(&input, &output) {
            Ok(report) => println!("{report}"),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        Ok(cli::Cli::Export(opts)) => match cli::export_signals_csv(&opts) {
            Ok(report) => println!("{report}"),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        Ok(cli::Cli::KvaserProbe) => match cli::kvaser_probe() {
            Ok(report) => println!("{report}"),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        Ok(cli::Cli::VectorProbe { fibex }) => match cli::vector_probe(fibex.as_deref()) {
            Ok(report) => println!("{report}"),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        Ok(cli::Cli::VectorTxProbe { channel }) => {
            match hw::vector::VectorChannel::tx_event_dump(channel, 500, 3, 1_000) {
                Ok(report) => println!("{report}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Err(msg) => {
            eprintln!("{msg}\n\n{}", cli::usage());
            std::process::exit(2);
        }
    }
}

/// A double-clicked exe owns its console, so an error would disappear with the
/// window. Bare launches pause before closing; `--web` scripts do not.
fn run_web(args: &[String], open: bool) {
    if let Err(error) = web::run(args, open) {
        eprintln!("error: {error}");
        if open && std::io::stdin().is_terminal() {
            eprintln!("Press Enter to close this window.");
            let _ = std::io::stdin().read_line(&mut String::new());
        }
        std::process::exit(1);
    }
}
