use anyhow::Result;
use std::sync::Arc;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;

// Create a global logger instance
lazy_static::lazy_static! {
  static ref LOGGER: Arc<Mutex<Option<BufWriter<File>>>> = Arc::new(Mutex::new(None));
}

pub async fn file_logger_initialize() -> Result<()> {
    let file = OpenOptions::new().append(true).create(true).open(".logs/output.log").await?;
    let writer = BufWriter::new(file);
    let mut logger = LOGGER.lock().await;

    if logger.is_none() {
        *logger = Some(writer);
    }

    Ok(())
}

pub async fn log_to_file(message: &str) -> Result<()> {
    let mut logger = LOGGER.lock().await;

    if let Some(writer) = logger.as_mut() {
        writer.write_all(format!("{}\n", message).as_bytes()).await?;
        writer.flush().await?; // TODO remove and use flush in main
    }

    Ok(())
}

// Call this function before exiting the application
pub async fn file_logger_flush() -> Result<()> {
    let mut logger = LOGGER.lock().await;

    if let Some(writer) = logger.as_mut() {
        writer.flush().await?;
    }

    Ok(())
}

mod tests {
    

    // #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[tokio::test]
    async fn test_custom_file_logger() -> Result<()> {
        file_logger_initialize().await?;

        log_to_file("Hello, world!").await?;

        // file_logger_flush().await?;

        Ok(())
    }
}
