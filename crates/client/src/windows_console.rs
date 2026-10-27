//! Windows console modes for `sherpa ssh`: virtual-terminal input, so keys arrive as the VT
//! byte sequences a remote shell expects, and virtual-terminal output, so the console draws
//! the escape sequences the shell sends.

use std::io;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Console::{
    CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    GetConsoleMode, GetStdHandle, STD_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode,
};

/// The console modes before [`enable_virtual_terminal`], for [`restore`].
pub struct ConsoleModes {
    input: CONSOLE_MODE,
    output: CONSOLE_MODE,
}

/// Turn on virtual-terminal input and output, keeping the other mode flags.
pub fn enable_virtual_terminal() -> io::Result<ConsoleModes> {
    let input = mode(STD_INPUT_HANDLE)?;
    let output = mode(STD_OUTPUT_HANDLE)?;
    set_mode(STD_INPUT_HANDLE, input | ENABLE_VIRTUAL_TERMINAL_INPUT)?;
    if let Err(error) = set_mode(
        STD_OUTPUT_HANDLE,
        output | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    ) {
        if let Err(restore) = set_mode(STD_INPUT_HANDLE, input) {
            tracing::warn!(error = %restore, "Failed to restore the console input mode");
        }
        return Err(error);
    }
    Ok(ConsoleModes { input, output })
}

/// Put back the modes [`enable_virtual_terminal`] found.
pub fn restore(modes: &ConsoleModes) -> io::Result<()> {
    set_mode(STD_INPUT_HANDLE, modes.input)?;
    set_mode(STD_OUTPUT_HANDLE, modes.output)
}

fn handle(which: STD_HANDLE) -> io::Result<HANDLE> {
    // SAFETY: `GetStdHandle` takes no pointers and only reads the process's standard handles.
    let handle = unsafe { GetStdHandle(which) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(handle)
}

fn mode(which: STD_HANDLE) -> io::Result<CONSOLE_MODE> {
    let handle = handle(which)?;
    let mut mode: CONSOLE_MODE = 0;
    // SAFETY: `handle` is a standard handle of this process and `mode` is a valid, writable
    // `CONSOLE_MODE` for the duration of the call.
    if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(mode)
}

fn set_mode(which: STD_HANDLE, mode: CONSOLE_MODE) -> io::Result<()> {
    let handle = handle(which)?;
    // SAFETY: `handle` is a standard handle of this process; `SetConsoleMode` takes the mode
    // by value.
    if unsafe { SetConsoleMode(handle, mode) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
