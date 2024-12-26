use lazy_static::lazy_static;
use one_common::custom_logger::*;
use tracing::Level;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};

lazy_static! {
    pub static ref GLOBAL_LOGGER: (CustomLogger, WorkerGuard) = CustomLogger::new(".logs", "customize.log");
}
pub async fn log_info_to_file(msg: &str) {
    GLOBAL_LOGGER.0.log_to_file(Level::INFO, msg).await;
}

mod tests {
    use super::*;

    #[tokio::test]
    async fn test_log_info_to_file() {
        log_info_to_file("log info message to the file.===").await;
    }
}
