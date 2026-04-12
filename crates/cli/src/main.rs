use agentdb_core::{ExitCode, VERSION};

fn main() {
    std::process::exit(run().as_i32());
}

fn run() -> ExitCode {
    let mut args = std::env::args().skip(1);

    match args.next().as_deref() {
        Some("init") => {
            println!("agentdb init is not implemented yet");
            ExitCode::Ok
        }
        Some("dev") => {
            println!("agentdb dev is not implemented yet");
            ExitCode::Ok
        }
        Some("doctor") => {
            println!("agentdb doctor is not implemented yet");
            ExitCode::Ok
        }
        Some("--version") | Some("version") => {
            println!("agentdb {}", VERSION);
            ExitCode::Ok
        }
        Some(command) => {
            eprintln!("unknown command: {command}");
            print_usage();
            ExitCode::Usage
        }
        None => {
            print_usage();
            ExitCode::Ok
        }
    }
}

fn print_usage() {
    println!("agentdb <command>");
    println!();
    println!("Commands:");
    println!("  init");
    println!("  dev");
    println!("  doctor");
    println!("  version");
}

