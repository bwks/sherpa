//! The exit status of a failed command, chosen from the error's type, never its text.

use anyhow::Error;

use shared::error::RpcErrorCode;

use super::ShellExit;
use crate::ws_client::messages::RpcFailure;

/// The command failed.
pub const FAILED: u8 = 1;
/// The server refused to start VMs or unikernels that do not fit in its memory now. Nothing was
/// started, and the same command can succeed once other nodes stop: sysexits' `EX_TEMPFAIL`.
pub const MEMORY_REFUSED: u8 = 75;

/// The exit status for `error`: the node shell's for `sherpa ssh`, [`MEMORY_REFUSED`] when the
/// server refused for memory, otherwise [`FAILED`].
pub fn exit_status(error: &Error) -> u8 {
    if let Some(ShellExit(code)) = error.downcast_ref::<ShellExit>() {
        return *code;
    }
    match error.downcast_ref::<RpcFailure>() {
        Some(failure) if failure.code == RpcErrorCode::InsufficientMemory => MEMORY_REFUSED,
        _ => FAILED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws_client::messages::RpcError;

    fn server_error(code: RpcErrorCode) -> Error {
        RpcError {
            code,
            message: "refused".to_string(),
            context: None,
        }
        .into_error("Lab creation failed")
    }

    #[test]
    fn a_memory_refusal_exits_75_through_added_context() {
        let error = server_error(RpcErrorCode::InsufficientMemory).context("Up failed");
        assert_eq!(exit_status(&error), MEMORY_REFUSED);
        assert_eq!(MEMORY_REFUSED, 75);
    }

    #[test]
    fn other_server_errors_exit_1() {
        for code in [
            RpcErrorCode::ServerError,
            RpcErrorCode::AccessDenied,
            RpcErrorCode::InvalidParams,
        ] {
            assert_eq!(exit_status(&server_error(code)), FAILED, "{code}");
        }
    }

    #[test]
    fn memory_text_alone_does_not_make_a_refusal() {
        let error = anyhow::anyhow!("Memory admission failed: Not enough memory on the server");
        assert_eq!(exit_status(&error), FAILED);
    }

    #[test]
    fn a_node_shell_exit_keeps_its_code() {
        assert_eq!(exit_status(&ShellExit(75).into()), 75);
        assert_eq!(exit_status(&ShellExit(2).into()), 2);
    }
}
