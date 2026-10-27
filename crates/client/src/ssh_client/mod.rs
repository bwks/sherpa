//! Connecting to lab nodes through the Sherpa server with the Jato SSH library, for
//! `sherpa ssh` and the SSH provisioners: the server host key prompt, and passphrase and
//! password prompts on the terminal, answered once per run.
mod connect;
mod prompter;

pub use connect::{PromptScope, connect, retryable};
pub use prompter::PromptCache;
