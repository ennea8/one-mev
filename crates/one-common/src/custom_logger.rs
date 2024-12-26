use std::io::Write;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::Level;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
// use tracing_subscriber::fmt::Layer;
// use tracing_subscriber::prelude::*;
// use tracing_subscriber::{EnvFilter, Registry};

pub struct CustomLogger {
    file_writer: Arc<Mutex<NonBlocking>>,
}

impl CustomLogger {
    pub fn new(log_dir: &str, log_file: &str) -> (Self, WorkerGuard) {
        let file_appender = RollingFileAppender::new(Rotation::DAILY, log_dir, log_file);
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

        (
            Self {
                file_writer: Arc::new(Mutex::new(non_blocking)),
            },
            guard,
        )
    }

    pub async fn log_to_file(&self, level: Level, message: &str) {
        let mut writer = self.file_writer.lock().await;
        let _ = writeln!(writer, "[{}] {}", level, message);
    }
}
mod tests {
    use super::*;
    use serde::{Serialize, Deserialize};

    #[tokio::test]
    async fn test_custom_logger() {
        // Create an instance of CustomLogger
        let (custom_logger, _custom_guard) = CustomLogger::new(".logs", "custom.log");

        // Log a custom message to the file asynchronously
        custom_logger
            .log_to_file(Level::INFO, "log message to the file.")
            .await;

        // log a struct data as json
        #[derive(Serialize)]
        struct MyStruct {
            field1: String,
            field2: u64,
        }
        let data = MyStruct {
            field1: "value1".to_string(),
            field2: 2,
        };
        custom_logger.log_to_file(Level::INFO, &serde_json::to_string(&data).unwrap()).await;

    }
}
