"""Send LLDP on interfaces and report the neighbours heard on them (Python 3, stdlib only).

Runs inside a lab VM as root, on the member ports of a Linux bond, standing in for an LLDP agent
the guest does not have. `<seconds> <system name> <interface>=<port id> ...` sends an LLDPDU
on each interface every few seconds, naming the system and the port, and listens for the same
time. It prints one line per interface it heard a neighbour on: `<interface> <system name>
<port id>`. Used by p2p_links_lab_test.py.
"""

import select
import socket
import struct
import sys
import time

DESTINATION = bytes.fromhex("0180c200000e")
ETH_P_ALL = 0x0003
ETH_P_LLDP = 0x88CC
TTL_SECONDS = 120
SEND_INTERVAL = 4
CHASSIS_LOCAL = 7
PORT_INTERFACE_NAME = 5
TLV_END, TLV_CHASSIS, TLV_PORT, TLV_TTL, TLV_SYSTEM_NAME = 0, 1, 2, 3, 5


def tlv(kind, value):
    return struct.pack("!H", (kind << 9) | len(value)) + value


def lldpdu(source_mac, system_name, port_id):
    """An LLDP frame from `source_mac` naming the system and the port."""
    body = tlv(TLV_CHASSIS, bytes([CHASSIS_LOCAL]) + system_name.encode())
    body += tlv(TLV_PORT, bytes([PORT_INTERFACE_NAME]) + port_id.encode())
    body += tlv(TLV_TTL, struct.pack("!H", TTL_SECONDS))
    body += tlv(TLV_SYSTEM_NAME, system_name.encode())
    body += tlv(TLV_END, b"")
    return DESTINATION + source_mac + struct.pack("!H", ETH_P_LLDP) + body


def neighbour(frame):
    """(system name, port id) from an LLDP frame, or None when it is not one. Without a system
    name the chassis ID stands in, as LLDP agents show it."""
    if len(frame) < 14 or struct.unpack("!H", frame[12:14])[0] != ETH_P_LLDP:
        return None
    values = {}
    offset = 14
    while offset + 2 <= len(frame):
        header = struct.unpack("!H", frame[offset:offset + 2])[0]
        kind, length = header >> 9, header & 0x1FF
        if kind == TLV_END:
            break
        values.setdefault(kind, frame[offset + 2:offset + 2 + length])
        offset += 2 + length
    if TLV_PORT not in values or not values[TLV_PORT]:
        return None
    port = values[TLV_PORT][1:].decode(errors="replace")
    system = values.get(TLV_SYSTEM_NAME) or values.get(TLV_CHASSIS, b"\x00")[1:]
    return system.decode(errors="replace"), port


def exchange(seconds, system_name, ports):
    """Send and listen on each interface of {interface: port id}; return {interface: neighbour}."""
    sockets = {}
    for interface in ports:
        sock = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(ETH_P_ALL))
        sock.bind((interface, 0))
        sockets[sock] = interface
    heard = {}
    try:
        end = time.monotonic() + seconds
        next_send = 0.0
        while (now := time.monotonic()) < end:
            if now >= next_send:
                for sock, interface in sockets.items():
                    with open(f"/sys/class/net/{interface}/address") as address:
                        mac = bytes.fromhex(address.read().strip().replace(":", ""))
                    sock.send(lldpdu(mac, system_name, ports[interface]))
                next_send = now + SEND_INTERVAL
            readable, _, _ = select.select(list(sockets), [], [], 0.5)
            for sock in readable:
                data, address = sock.recvfrom(65535)
                if address[2] == socket.PACKET_OUTGOING:
                    continue
                found = neighbour(data)
                if found:
                    heard[sockets[sock]] = found
    finally:
        for sock in sockets:
            sock.close()
    return heard


if __name__ == "__main__":
    pairs = dict(argument.split("=", 1) for argument in sys.argv[3:])
    for interface, (system, port) in sorted(exchange(float(sys.argv[1]), sys.argv[2], pairs).items()):
        print(f"{interface} {system} {port}", flush=True)
