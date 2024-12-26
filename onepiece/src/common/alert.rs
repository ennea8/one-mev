use anyhow::Result;


pub struct Alert {

}

impl Alert {
  pub fn new() -> Self {
    Alert{}
  }
  pub async fn send(&self, message: &str) -> Result<()> {

    Ok(())
  }
}