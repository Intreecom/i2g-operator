use clap::Parser;

use crate::args::I2GArgs;

#[derive(Clone)]
pub struct Context {
    pub args: I2GArgs,
    pub client: kube::Client,
}

impl Context {
    pub async fn new() -> anyhow::Result<Self> {
        let args = I2GArgs::parse();
        let client = kube::Client::try_default().await?;
        Ok(Context { args, client })
    }
}
