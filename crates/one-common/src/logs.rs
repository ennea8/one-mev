use std::sync::{Once, OnceLock};
use tracing_appender::non_blocking;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

use colored::Colorize;
use indoc::indoc;

static INIT_A: Once = Once::new();
pub fn init_logs() {
    INIT_A.call_once(|| {
        let _ = tracing_subscriber::fmt::try_init();
    });
}

static INIT: OnceLock<WorkerGuard> = OnceLock::new();
// static INIT: Once = Once::new();

pub fn init_logs_v2() -> &'static WorkerGuard {
    INIT.get_or_init(|| {
        // File appender for logging errors
        let file_appender = tracing_appender::rolling::daily(".logs", "error.log");
        let (file_writer, guard) = non_blocking(file_appender);

        // let env_filter = EnvFilter::builder()
        // .with_default_directive("error".parse().unwrap())
        // .with_env_var("RUST_LOG")
        // .from_env_lossy();

        // Set up a filter using RUST_LOG environment variable, defaulting to "debug" for console
        let console_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info") // default to "debug" if RUST_LOG is not set
        });

        // Set up a filter using RUST_LOG_FILE environment variable for file logging
        let file_filter = std::env::var("RUST_LOG_FILE")
            .map(EnvFilter::new)
            .unwrap_or_else(|_| EnvFilter::new("error")); // default to "error" if RUST_LOG_FILE is not set

        // Setting up logging to the console
        let console_layer = fmt::layer()
            .with_writer(std::io::stdout)
            .with_filter(console_filter);

        // Setting up logging to the file for errors only
        let file_layer = fmt::layer()
            .with_writer(file_writer)
            .with_filter(file_filter);

        // Combine the console and file layers
        let result = tracing_subscriber::registry()
            .with(console_layer)
            .with(file_layer)
            .try_init();

        match result {
            Ok(_) => {
                info!("Logging initialized");
            }
            Err(e) => {
                eprintln!("Error initializing logging: {:?}", e);
            }
        }
        guard
    })
}

pub fn print_banner() {
    let banner = indoc! {
r#"
                         _                                                               
                        (_)                
  ___  ____   ____ ____  _  ____ ____ ____ 
 / _ \|  _ \ / _  )  _ \| |/ _  ) ___) _  )
| |_| | | | ( (/ /| | | | ( (/ ( (__( (/ / 
 \___/|_| |_|\____) ||_/|_|\____)____)____)
                  |_|                                                                                                                   
                                              
"#};

    println!("{}", format!("{}", banner.bright_green().bold()));
}
mod tests {
    use super::*;

    #[test]
    fn test_init_logs() {
        println!("RUST_LOG logs: {}", std::env::var("RUST_LOG").unwrap());

        init_logs();

        info!("info");
        warn!("warn");
        error!("error");
        debug!("debug");
        trace!("trace");

        println!("hello");
    }

    #[test]
    fn test_init_logs_v2() {
        println!("RUST_LOG logs_v2: {}", std::env::var("RUST_LOG").unwrap());

        let _log = init_logs_v2();

        info!("info");
        warn!("warn");
        error!("error");
        debug!("debug");
        trace!("trace");

        println!("hello");
    }

    #[test]
    fn test_print_banner() {
        print_banner();
    }
}
