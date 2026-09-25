use tunic_app::{AppConfig, default_data_directory};

fn main() {
    let result = default_data_directory()
        .map(|data_directory| AppConfig { data_directory })
        .and_then(tunic_app::run);
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
