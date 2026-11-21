//! The exit status of a failed command, chosen from the error's type, never its text.

use anyhow::Error;

use shared::error::RpcErrorCode;

use super::ShellExit;
use crate::ws_client::messages::RpcFailure;

/// The command failed.
pub const FAILED: u8 = 1;
/// The server refused to start nodes for now: they do not fit in its memory, or host CPUs they
/// pin are held by another node. Nothing was started, and the same command can succeed once
/// other nodes stop or let the CPUs go: sysexits' `EX_TEMPFAIL`.
pub const TRY_LATER: u8 = 75;

/// The exit status for `error`: the node shell's for `sherpa ssh`, [`TRY_LATER`] when the server
/// refused for memory or for CPU pins held, otherwise [`FAILED`].
pub fn exit_status(error: &Error) -> u8 {
    if let Some(ShellExit(code)) = error.downcast_ref::<ShellExit>() {
        return *code;
    }
    match error.downcast_ref::<RpcFailure>() {
        Some(failure)
            if matches!(
                failure.code,
                RpcErrorCode::InsufficientMemory | RpcErrorCode::CpuPinsHeld
            ) =>
        {
            TRY_LATER
        }
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
        assert_eq!(exit_status(&error), TRY_LATER);
        assert_eq!(TRY_LATER, 75);
    }

    #[test]
    fn a_cpu_pin_refusal_exits_75_through_added_context() {
        let error = server_error(RpcErrorCode::CpuPinsHeld).context("Up failed");
        assert_eq!(exit_status(&error), TRY_LATER);
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
    fn refusal_text_alone_does_not_make_a_refusal() {
        for text in [
            "Memory admission failed: Not enough memory on the server",
            "CPU pinning validation failed: Host CPU 2 is held",
        ] {
            assert_eq!(exit_status(&anyhow::anyhow!(text)), FAILED, "{text}");
        }
    }

    #[test]
    fn ssh_target_008_a_node_shell_s_exit_code_is_the_exit_code_of_sherpa_ssh() {
        assert_eq!(exit_status(&ShellExit(75).into()), 75);
        assert_eq!(exit_status(&ShellExit(2).into()), 2);
    }
}
