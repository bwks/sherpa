//! OpenSSH's `~.` escape: typed at the start of a line, it ends the session even when the
//! node no longer answers. `~~` sends a single `~`; a `~` followed by any other key is sent
//! as typed.

const ESCAPE: u8 = b'~';
const DISCONNECT: u8 = b'.';

/// Watches typed bytes for the escape, across reads.
pub(super) struct EscapeFilter {
    line_start: bool,
    pending_escape: bool,
}

/// What to send for one read of typed bytes, and whether the escape asked to disconnect.
/// Bytes typed after the escape in the same read are dropped.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Filtered {
    pub send: Vec<u8>,
    pub disconnect: bool,
}

impl EscapeFilter {
    pub(super) fn new() -> Self {
        Self {
            line_start: true,
            pending_escape: false,
        }
    }

    pub(super) fn filter(&mut self, input: &[u8]) -> Filtered {
        let mut send = Vec::with_capacity(input.len());
        for &byte in input {
            if self.pending_escape {
                self.pending_escape = false;
                match byte {
                    DISCONNECT => {
                        return Filtered {
                            send,
                            disconnect: true,
                        };
                    }
                    ESCAPE => {
                        send.push(ESCAPE);
                        self.line_start = false;
                        continue;
                    }
                    _ => send.push(ESCAPE),
                }
            } else if self.line_start && byte == ESCAPE {
                self.pending_escape = true;
                continue;
            }
            send.push(byte);
            self.line_start = byte == b'\r' || byte == b'\n';
        }
        Filtered {
            send,
            disconnect: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter_all(chunks: &[&[u8]]) -> (Vec<u8>, bool) {
        let mut escape = EscapeFilter::new();
        let mut sent = Vec::new();
        for chunk in chunks {
            let filtered = escape.filter(chunk);
            sent.extend(filtered.send);
            if filtered.disconnect {
                return (sent, true);
            }
        }
        (sent, false)
    }

    #[test]
    fn tilde_dot_at_the_start_disconnects() {
        assert_eq!(filter_all(&[b"~."]), (vec![], true));
    }

    #[test]
    fn tilde_dot_after_enter_disconnects_after_sending_the_line() {
        assert_eq!(filter_all(&[b"ls\r~."]), (b"ls\r".to_vec(), true));
    }

    #[test]
    fn tilde_dot_split_across_reads_disconnects() {
        assert_eq!(filter_all(&[b"\r~", b"."]), (b"\r".to_vec(), true));
    }

    #[test]
    fn tilde_dot_inside_a_line_is_typed() {
        assert_eq!(filter_all(&[b"cd ~.ssh"]), (b"cd ~.ssh".to_vec(), false));
    }

    #[test]
    fn double_tilde_sends_one() {
        assert_eq!(filter_all(&[b"~~."]), (b"~.".to_vec(), false));
    }

    #[test]
    fn tilde_then_another_key_sends_both() {
        assert_eq!(filter_all(&[b"~/bin\r"]), (b"~/bin\r".to_vec(), false));
    }

    #[test]
    fn newline_also_starts_a_line() {
        assert_eq!(filter_all(&[b"a\n~."]), (b"a\n".to_vec(), true));
    }
}
