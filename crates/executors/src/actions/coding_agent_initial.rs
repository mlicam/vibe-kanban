use std::path::PathBuf;

use async_trait::async_trait;
use command_group::AsyncGroupChild;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    actions::Executable,
    command::ProfileVariant,
    executors::{CodingAgent, ExecutorError, StandardCodingAgentExecutor},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
pub struct CodingAgentInitialRequest {
    pub prompt: String,
    pub profile: ProfileVariant,
}

#[async_trait]
impl Executable for CodingAgentInitialRequest {
    async fn spawn(&self, current_dir: &PathBuf) -> Result<AsyncGroupChild, ExecutorError> {
        let executor = CodingAgent::from_profile_variant(&self.profile)?;
        executor.spawn(current_dir, &self.prompt).await
    }
}
