#!/usr/bin/env python3
"""Send or receive a fixed set of marked L2 frames on an interface (Python 3, stdlib only).

Runs inside a lab VM (as root) or a sidecar container sharing a container node's network
namespace. `send <interface> <sender>` sends one frame of each kind, marked with the sender's
name. `receive <interface> <seconds>` prints one line per marked frame it received:
`<sender> <kind> <length> <tag>`, where `tag` is the 802.1Q/802.1ad tag the kernel moved into
packet metadata (`-` when there is none). Used by p2p_links_lab_test.py.
"""

import socket
import struct
import sys
import time

SOURCE_MAC = bytes.fromhex("020000391001")
MARK = b"SHERPA391:"
ETH_P_ALL = 0x0003
SOL_PACKET = 263
PACKET_AUXDATA = 8
TP_STATUS_VLAN_VALID = 0x10
# The frame kinds, each with its destination MAC and what follows the source MAC up to the mark.
KINDS = (
    ("lldp", "0180c200000e", "88cc"),
    ("stp", "0180c2000000", "0016424203"),
    ("pause", "0180c2000001", "88080001ffff"),
    ("lacp", "0180c2000002", "88090101"),
    ("eapol", "0180c2000003", "888e"),
    ("garp", "0180c2000020", "88b5"),
    ("cdp", "01000ccccccc", "0016aaaa0300000c2000"),
    ("vlan10", "ffffffffffff", "8100000a88b5"),
    ("qinq100.10", "ffffffffffff", "88a800648100000a88b5"),
    ("jumbo9000", "ffffffffffff", "88b5"),
)
JUMBO_LENGTH = 9014


def frame(sender, kind, destination, header):
    body = bytes.fromhex(destination) + SOURCE_MAC + bytes.fromhex(header)
    body += MARK + f"{sender}:{kind}".encode() + b"\x00"
    if kind.startswith("jumbo"):
        body += b"\x00" * (JUMBO_LENGTH - len(body))
    return body


def send(interface, sender):
    with socket.socket(socket.AF_PACKET, socket.SOCK_RAW) as sock:
        sock.bind((interface, 0))
        for kind, destination, header in KINDS:
            sock.send(frame(sender, kind, destination, header))
            time.sleep(0.05)


def offloaded_tag(ancillary):
    for level, kind, data in ancillary:
        if level == SOL_PACKET and kind == PACKET_AUXDATA and len(data) >= 20:
            status, _, _, _, _, tci, tpid = struct.unpack("IIIHHHH", data[:20])
            if status & TP_STATUS_VLAN_VALID:
                return f"0x{tpid:04x}/{tci & 0xfff}"
    return "-"


def receive(interface, seconds):
    with socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(ETH_P_ALL)) as sock:
        sock.bind((interface, 0))
        sock.setsockopt(SOL_PACKET, PACKET_AUXDATA, 1)
        sock.settimeout(0.5)
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            try:
                data, ancillary, _, address = sock.recvmsg(65535, socket.CMSG_SPACE(20))
            except socket.timeout:
                continue
            if address[2] == socket.PACKET_OUTGOING or MARK not in data:
                continue
            sender, kind = data[data.index(MARK) + len(MARK):].split(b"\x00")[0].decode().split(":")
            print(f"{sender} {kind} {len(data)} {offloaded_tag(ancillary)}", flush=True)


if __name__ == "__main__":
    if sys.argv[1] == "send":
        send(sys.argv[2], sys.argv[3])
    else:
        receive(sys.argv[2], float(sys.argv[3]))
