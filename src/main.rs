use std::process::ExitCode;

fn main() -> ExitCode {
    match skillnet::cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if let Some(exit) = error.downcast_ref::<skillnet::exit::ExitError>() {
                if exit.code() != 4 {
                    eprintln!("Error: {exit}");
                }
                ExitCode::from(exit.code())
            } else {
                eprintln!("Error: {error:?}");
                ExitCode::FAILURE
            }
        }
    }
}
