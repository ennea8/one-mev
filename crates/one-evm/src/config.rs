use std::path::{Path, PathBuf};

pub fn cache_dir() -> PathBuf {
  std::env::var("REVM_CACHE_DIR").map(PathBuf::from).unwrap_or_else(|_| {
      let default_path = "./.revm_cache";
      Path::new(default_path).to_path_buf()
  })
}