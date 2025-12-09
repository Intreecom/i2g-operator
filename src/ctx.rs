use std::sync::{Arc, atomic::AtomicBool};

use clap::Parser;

use crate::args::I2GArgs;

#[derive(Clone)]
pub struct Context {
    pub args: I2GArgs,
    pub client: kube::Client,
    pub is_leader: Arc<AtomicBool>,
    pub hostname: String,
}

impl Context {
    pub async fn new() -> anyhow::Result<Self> {
        let args = I2GArgs::parse();
        let client = kube::Client::try_default().await?;
        let is_leader = Arc::new(AtomicBool::new(false));
        let hostname = std::env::var("HOSTNAME").or_else(|_| std::env::var("HOST"))?;
        Ok(Context {
            args,
            client,
            is_leader,
            hostname,
        })
    }
}
