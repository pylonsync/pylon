use pylon_kernel::{ExitCode, VERSION};

use crate::output::print_json;

pub fn run(json_mode: bool) -> ExitCode {
    if json_mode {
        print_json(&serde_json::json!({ "version": VERSION }));
    } else {
        println!("pylon {VERSION}");
    }
    ExitCode::Ok
}
