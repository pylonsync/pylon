use statecraft_core::{ExitCode, VERSION};

use crate::output::print_json;

pub fn run(json_mode: bool) -> ExitCode {
    if json_mode {
        print_json(&serde_json::json!({ "version": VERSION }));
    } else {
        println!("statecraft {VERSION}");
    }
    ExitCode::Ok
}
