#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// The core keeps observer and project-file models that the retired desktop UI
// used to populate. They still participate in project compatibility and tests;
// pruning them is separate from the web migration.
#![allow(dead_code)]

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
mod eth;
mod export;
mod fr_db;
mod generator;
mod hw;
mod load;
mod log;
mod node;
mod observe;
mod profile;
mod project;
mod recorder;
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
        cli::attach_parent_console();
        let open = std::env::var_os("ROXY_WEB_NO_OPEN").is_none();
        web::run(&args, open);
        return;
    }
    if args.iter().any(|arg| arg == "--web") {
        cli::attach_parent_console();
        web::run(&args, false);
        return;
    }
    match cli::parse_args(&args) {
        Ok(cli::Cli::Gui) => {
            cli::attach_parent_console();
            println!("{}", cli::usage());
        }
        Ok(cli::Cli::Help(text)) => {
            cli::attach_parent_console();
            println!("{text}");
        }
        Ok(cli::Cli::Run(opts)) => {
            cli::attach_parent_console();
            match cli::run(&opts) {
                Ok(report) => println!("{report}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Ok(cli::Cli::CheckScripts(check)) => {
            cli::attach_parent_console();
            match cli::check_scripts(&check) {
                Ok(report) => println!("{report}"),
                Err(report) => {
                    eprintln!("{report}");
                    std::process::exit(2);
                }
            }
        }
        Ok(cli::Cli::Convert(input, output)) => {
            cli::attach_parent_console();
            match cli::convert_log(&input, &output) {
                Ok(report) => println!("{report}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Ok(cli::Cli::Export(opts)) => {
            cli::attach_parent_console();
            match cli::export_signals_csv(&opts) {
                Ok(report) => println!("{report}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Ok(cli::Cli::KvaserProbe) => {
            cli::attach_parent_console();
            match cli::kvaser_probe() {
                Ok(report) => println!("{report}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Ok(cli::Cli::VectorProbe { fibex }) => {
            cli::attach_parent_console();
            match cli::vector_probe(fibex.as_deref()) {
                Ok(report) => println!("{report}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Ok(cli::Cli::VectorTxProbe { channel }) => {
            cli::attach_parent_console();
            match hw::vector::VectorChannel::tx_event_dump(channel, 500, 3, 1_000) {
                Ok(report) => println!("{report}"),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Err(msg) => {
            cli::attach_parent_console();
            eprintln!("{msg}\n\n{}", cli::usage());
            std::process::exit(2);
        }
    }
}
