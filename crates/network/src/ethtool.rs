//! Interface offload features through the kernel's ethtool netlink interface, the generic
//! netlink family `ethtool`.
//!
//! Transmit checksum offload can be read, and one change is made: turning it off, which also takes segmentation
//! offload with it. A veth with it on hands the peer a TCP or UDP packet whose checksum is left
//! for "hardware" to fill in. A kernel stack accepts that; a program reading the peer through a
//! packet socket, such as the IOL bridge, gets the unfinished packet. With it off, the kernel
//! finishes the checksum before the veth sends the packet.
//!
//! The messages are built here byte by byte; the socket is netlink-sys, through rtnetlink.

use anyhow::{Context, Result, bail};
use rtnetlink::sys::constants::NETLINK_GENERIC;
use rtnetlink::sys::{Socket, SocketAddr};
use tracing::instrument;

const NLMSG_HEADER_LEN: usize = 16;
const GENL_HEADER_LEN: usize = 4;
const NLA_HEADER_LEN: usize = 4;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_ACK: u16 = 0x4;
const NLA_F_NESTED: u16 = 0x8000;
/// Mask of the attribute type, without the nested and byte order flags.
const NLA_TYPE_MASK: u16 = 0x3fff;

/// The generic netlink controller, which resolves family names to IDs.
const GENL_ID_CTRL: u16 = 0x10;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;

const ETHTOOL_GENL_NAME: &str = "ethtool";
const ETHTOOL_GENL_VERSION: u8 = 1;
const ETHTOOL_MSG_FEATURES_GET: u8 = 11;
const ETHTOOL_MSG_FEATURES_SET: u8 = 12;
/// The kernel's reply to `ETHTOOL_MSG_FEATURES_GET`, numbered in the kernel's own sequence.
const ETHTOOL_MSG_FEATURES_GET_REPLY: u8 = 11;
const ETHTOOL_A_FEATURES_HEADER: u16 = 1;
const ETHTOOL_A_FEATURES_WANTED: u16 = 3;
const ETHTOOL_A_FEATURES_ACTIVE: u16 = 4;
const ETHTOOL_A_HEADER_DEV_INDEX: u16 = 1;
const ETHTOOL_A_HEADER_FLAGS: u16 = 3;
/// No reply beyond the acknowledgement.
const ETHTOOL_FLAG_OMIT_REPLY: u32 = 1 << 1;
/// A bitset with no mask: it lists only the bits that are set.
const ETHTOOL_A_BITSET_NOMASK: u16 = 1;
const ETHTOOL_A_BITSET_BITS: u16 = 3;
const ETHTOOL_A_BITSET_BITS_BIT: u16 = 1;
const ETHTOOL_A_BITSET_BIT_NAME: u16 = 2;
/// In a bitset with a mask, the flag of a bit that is set.
const ETHTOOL_A_BITSET_BIT_VALUE: u16 = 3;

/// The transmit checksum features, as `ethtool -K <iface> tx off` turns off: a feature listed
/// with no value is turned off. A device that does not have one ignores it. Offload is on while
/// any of them is active.
pub(crate) const TX_CHECKSUM_FEATURES: &[&str] = &[
    "tx-checksum-ipv4",
    "tx-checksum-ip-generic",
    "tx-checksum-ipv6",
    "tx-checksum-fcoe-crc",
    "tx-checksum-sctp",
];

/// Sequence numbers of the two requests on one socket: the family lookup, then the features
/// request.
const FAMILY_SEQ: u32 = 1;
const FEATURES_SEQ: u32 = 2;

/// A netlink message under construction: a header, then attributes.
struct MessageBuilder {
    buf: Vec<u8>,
}

impl MessageBuilder {
    /// A generic netlink message of family `family`, command `cmd`, asking for an ACK.
    fn new(family: u16, seq: u32, cmd: u8, version: u8) -> Self {
        let mut buf = Vec::with_capacity(256);
        buf.extend_from_slice(&0u32.to_ne_bytes()); // length, set by `finish`
        buf.extend_from_slice(&family.to_ne_bytes());
        buf.extend_from_slice(&(NLM_F_REQUEST | NLM_F_ACK).to_ne_bytes());
        buf.extend_from_slice(&seq.to_ne_bytes());
        buf.extend_from_slice(&0u32.to_ne_bytes()); // port ID: the kernel fills it in
        buf.extend_from_slice(&[cmd, version, 0, 0]);
        Self { buf }
    }

    /// Add attribute `kind` holding `payload`, padded to four bytes.
    fn attr(&mut self, kind: u16, payload: &[u8]) -> Result<()> {
        let len =
            u16::try_from(NLA_HEADER_LEN + payload.len()).context("netlink attribute too long")?;
        self.buf.extend_from_slice(&len.to_ne_bytes());
        self.buf.extend_from_slice(&kind.to_ne_bytes());
        self.buf.extend_from_slice(payload);
        self.pad();
        Ok(())
    }

    /// Start nested attribute `kind`; `end_nested` with the returned offset closes it.
    fn start_nested(&mut self, kind: u16) -> usize {
        let start = self.buf.len();
        self.buf.extend_from_slice(&0u16.to_ne_bytes());
        self.buf
            .extend_from_slice(&(kind | NLA_F_NESTED).to_ne_bytes());
        start
    }

    fn end_nested(&mut self, start: usize) -> Result<()> {
        let len = u16::try_from(self.buf.len() - start).context("netlink attribute too long")?;
        self.buf[start..start + 2].copy_from_slice(&len.to_ne_bytes());
        Ok(())
    }

    fn pad(&mut self) {
        while !self.buf.len().is_multiple_of(4) {
            self.buf.push(0);
        }
    }

    fn finish(mut self) -> Result<Vec<u8>> {
        let len = u32::try_from(self.buf.len()).context("netlink message too long")?;
        self.buf[..4].copy_from_slice(&len.to_ne_bytes());
        Ok(self.buf)
    }
}

/// A NUL-terminated string attribute payload.
fn c_string(text: &str) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

/// The controller request for the ID of generic netlink family `name`.
fn family_request(name: &str) -> Result<Vec<u8>> {
    let mut message = MessageBuilder::new(GENL_ID_CTRL, FAMILY_SEQ, CTRL_CMD_GETFAMILY, 1);
    message.attr(CTRL_ATTR_FAMILY_NAME, &c_string(name))?;
    message.finish()
}

/// The ethtool request that turns `features` off on the interface with index `ifindex`.
fn features_off_request(family: u16, ifindex: u32, features: &[&str]) -> Result<Vec<u8>> {
    let mut message = MessageBuilder::new(
        family,
        FEATURES_SEQ,
        ETHTOOL_MSG_FEATURES_SET,
        ETHTOOL_GENL_VERSION,
    );
    let header = message.start_nested(ETHTOOL_A_FEATURES_HEADER);
    message.attr(ETHTOOL_A_HEADER_DEV_INDEX, &ifindex.to_ne_bytes())?;
    message.attr(
        ETHTOOL_A_HEADER_FLAGS,
        &ETHTOOL_FLAG_OMIT_REPLY.to_ne_bytes(),
    )?;
    message.end_nested(header)?;

    let wanted = message.start_nested(ETHTOOL_A_FEATURES_WANTED);
    let bits = message.start_nested(ETHTOOL_A_BITSET_BITS);
    for feature in features {
        let bit = message.start_nested(ETHTOOL_A_BITSET_BITS_BIT);
        message.attr(ETHTOOL_A_BITSET_BIT_NAME, &c_string(feature))?;
        message.end_nested(bit)?;
    }
    message.end_nested(bits)?;
    message.end_nested(wanted)?;
    message.finish()
}

/// The ethtool request for the features of the interface with index `ifindex`. With no header
/// flags the kernel replies, with each bitset listed by name.
fn features_get_request(family: u16, ifindex: u32) -> Result<Vec<u8>> {
    let mut message = MessageBuilder::new(
        family,
        FEATURES_SEQ,
        ETHTOOL_MSG_FEATURES_GET,
        ETHTOOL_GENL_VERSION,
    );
    let header = message.start_nested(ETHTOOL_A_FEATURES_HEADER);
    message.attr(ETHTOOL_A_HEADER_DEV_INDEX, &ifindex.to_ne_bytes())?;
    message.end_nested(header)?;
    message.finish()
}

/// One netlink message in a reply: its type, sequence number and payload after the header.
struct Reply<'a> {
    kind: u16,
    seq: u32,
    payload: &'a [u8],
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The messages in one datagram from the kernel.
fn replies(datagram: &[u8]) -> Result<Vec<Reply<'_>>> {
    let mut messages = Vec::new();
    let mut at = 0;
    while at + NLMSG_HEADER_LEN <= datagram.len() {
        let len = read_u32(datagram, at)
            .and_then(|len| usize::try_from(len).ok())
            .context("truncated netlink header")?;
        if len < NLMSG_HEADER_LEN || at + len > datagram.len() {
            bail!("malformed netlink message of length {len}");
        }
        messages.push(Reply {
            kind: read_u16(datagram, at + 4).context("truncated netlink header")?,
            seq: read_u32(datagram, at + 8).context("truncated netlink header")?,
            payload: &datagram[at + NLMSG_HEADER_LEN..at + len],
        });
        at += len.next_multiple_of(4);
    }
    Ok(messages)
}

/// The attributes in `bytes`, each as its type, without the flags, and its payload, without
/// padding. Parsing stops at a malformed attribute.
fn parse_attrs(bytes: &[u8]) -> Vec<(u16, &[u8])> {
    let mut attrs = Vec::new();
    let mut at = 0;
    while at + NLA_HEADER_LEN <= bytes.len() {
        let (Some(len), Some(kind)) = (read_u16(bytes, at), read_u16(bytes, at + 2)) else {
            break;
        };
        let len = usize::from(len);
        if len < NLA_HEADER_LEN || at + len > bytes.len() {
            break;
        }
        attrs.push((kind & NLA_TYPE_MASK, &bytes[at + NLA_HEADER_LEN..at + len]));
        at += len.next_multiple_of(4);
    }
    attrs
}

/// The payload of the first attribute of type `kind` in `attrs`.
fn attr_payload<'a>(attrs: &[(u16, &'a [u8])], kind: u16) -> Option<&'a [u8]> {
    attrs
        .iter()
        .find(|(found, _)| *found == kind)
        .map(|(_, payload)| *payload)
}

/// Whether a transmit checksum feature is set in the active features of a features reply,
/// `attrs` being the reply's attributes after its generic netlink header.
fn tx_checksum_active(attrs: &[u8]) -> Result<bool> {
    let active = attr_payload(&parse_attrs(attrs), ETHTOOL_A_FEATURES_ACTIVE)
        .context("the features reply has no active features")?;
    let bitset = parse_attrs(active);
    let unmasked = attr_payload(&bitset, ETHTOOL_A_BITSET_NOMASK).is_some();
    let bits = attr_payload(&bitset, ETHTOOL_A_BITSET_BITS)
        .context("the active features are not listed by name")?;
    Ok(parse_attrs(bits)
        .into_iter()
        .filter(|(kind, _)| *kind == ETHTOOL_A_BITSET_BITS_BIT)
        .any(|(_, bit)| {
            let bit = parse_attrs(bit);
            let set = unmasked || attr_payload(&bit, ETHTOOL_A_BITSET_BIT_VALUE).is_some();
            let name = attr_payload(&bit, ETHTOOL_A_BITSET_BIT_NAME)
                .and_then(|name| name.split(|byte| *byte == 0).next());
            set && name.is_some_and(|name| {
                TX_CHECKSUM_FEATURES
                    .iter()
                    .any(|feature| feature.as_bytes() == name)
            })
        }))
}

/// What a reply to request `seq` says: `Some(Ok)` for an ACK, `Some(Err)` for an error, `None`
/// for anything else.
fn acknowledgement(reply: &Reply<'_>, seq: u32) -> Option<Result<()>> {
    if reply.kind != NLMSG_ERROR || reply.seq != seq {
        return None;
    }
    let Some(code) = read_u32(reply.payload, 0) else {
        return Some(Err(anyhow::anyhow!("truncated netlink error message")));
    };
    // The kernel sends a negative errno, or 0 for an ACK.
    let errno = i32::from_ne_bytes(code.to_ne_bytes());
    Some(if errno == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(-errno).into())
    })
}

/// The family ID in the controller's reply to `family_request`.
fn family_id(reply: &Reply<'_>) -> Option<u16> {
    if reply.kind != GENL_ID_CTRL || reply.seq != FAMILY_SEQ {
        return None;
    }
    let attrs = parse_attrs(reply.payload.get(GENL_HEADER_LEN..)?);
    read_u16(attr_payload(&attrs, CTRL_ATTR_FAMILY_ID)?, 0)
}

/// The answer to the family request: the family's ID, or the error the kernel sent.
fn family_answer(reply: &Reply<'_>) -> Option<Result<u16>> {
    if let Some(id) = family_id(reply) {
        return Some(Ok(id));
    }
    match acknowledgement(reply, FAMILY_SEQ)? {
        Ok(()) => Some(Err(anyhow::anyhow!(
            "the kernel acknowledged the ethtool family lookup without its ID"
        ))),
        Err(error) => Some(Err(error)),
    }
}

/// The answer to the features request: its acknowledgement.
fn features_answer(reply: &Reply<'_>) -> Option<Result<()>> {
    acknowledgement(reply, FEATURES_SEQ)
}

/// The answer to the features read: whether transmit checksum offload is on, or the error the
/// kernel sent.
fn features_state(reply: &Reply<'_>) -> Option<Result<bool>> {
    if reply.seq != FEATURES_SEQ {
        return None;
    }
    if let Some(acknowledged) = acknowledgement(reply, FEATURES_SEQ) {
        return Some(acknowledged.and_then(|()| {
            bail!("the kernel acknowledged the features request without the features")
        }));
    }
    if reply.payload.first() != Some(&ETHTOOL_MSG_FEATURES_GET_REPLY) {
        return None;
    }
    Some(
        reply
            .payload
            .get(GENL_HEADER_LEN..)
            .context("truncated features reply")
            .and_then(tx_checksum_active),
    )
}

/// Send `request` and read replies until `accept` returns a value for one.
fn exchange<T>(
    socket: &Socket,
    request: &[u8],
    accept: fn(&Reply<'_>) -> Option<Result<T>>,
) -> Result<T> {
    socket
        .send(request, 0)
        .context("cannot send to ethtool netlink")?;
    loop {
        let (datagram, _) = socket
            .recv_from_full()
            .context("cannot read from ethtool netlink")?;
        for reply in replies(&datagram)? {
            if let Some(result) = accept(&reply) {
                return result;
            }
            if reply.kind == NLMSG_DONE {
                bail!("ethtool netlink ended without an answer");
            }
        }
    }
}

/// A generic netlink socket connected to the kernel, and the ID of the ethtool family on it.
fn ethtool_socket() -> Result<(Socket, u16)> {
    let mut socket =
        Socket::new(NETLINK_GENERIC).context("cannot open a generic netlink socket")?;
    socket
        .bind_auto()
        .context("cannot bind a generic netlink socket")?;
    socket
        .connect(&SocketAddr::new(0, 0))
        .context("cannot connect to the kernel's generic netlink")?;

    let family = exchange(&socket, &family_request(ETHTOOL_GENL_NAME)?, family_answer)
        .context("the kernel has no ethtool netlink family")?;
    Ok((socket, family))
}

/// Whether transmit checksum offload is on for the interface with index `ifindex`, in this
/// network namespace: whether any feature `disable_tx_checksum_offload` turns off is active.
///
/// # Errors
///
/// Fails when the kernel has no ethtool netlink family, or refuses the request, as for a
/// missing interface.
#[instrument(level = "debug")]
pub fn tx_checksum_offload_on(ifindex: u32) -> Result<bool> {
    let (socket, family) = ethtool_socket()?;
    exchange(
        &socket,
        &features_get_request(family, ifindex)?,
        features_state,
    )
    .with_context(|| format!("cannot read the transmit checksum offload of ifindex {ifindex}"))
}

/// Turn transmit checksum offload off on the interface with index `ifindex`, in this network
/// namespace. Turning it off when it is off already changes nothing.
///
/// # Errors
///
/// Fails when the kernel has no ethtool netlink family, or refuses the change, as for a
/// missing interface or without `CAP_NET_ADMIN`.
#[instrument(level = "debug")]
pub fn disable_tx_checksum_offload(ifindex: u32) -> Result<()> {
    let (socket, family) = ethtool_socket()?;
    let request = features_off_request(family, ifindex, TX_CHECKSUM_FEATURES)?;
    exchange(&socket, &request, features_answer).with_context(|| {
        format!("cannot turn transmit checksum offload off on ifindex {ifindex}")
    })?;
    tracing::debug!(ifindex, "Transmit checksum offload off");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{create_veth_pair, delete_interface, get_ifindex};

    /// Attributes the kernel puts in a bitset that the code here does not read.
    const ETHTOOL_A_BITSET_SIZE: u16 = 2;
    const ETHTOOL_A_BITSET_BIT_INDEX: u16 = 1;

    /// The attributes of a message, after its netlink and generic netlink headers.
    fn attributes(message: &[u8]) -> &[u8] {
        &message[NLMSG_HEADER_LEN + GENL_HEADER_LEN..]
    }

    /// The nested attributes of attribute `kind` at the top of `attrs`, and its type flags.
    fn first_attr(attrs: &[u8]) -> (u16, &[u8]) {
        let len = usize::from(read_u16(attrs, 0).unwrap());
        let kind = read_u16(attrs, 2).unwrap();
        (kind, &attrs[NLA_HEADER_LEN..len])
    }

    #[test]
    fn a_family_request_asks_the_controller_for_the_name() {
        let message = family_request("ethtool").unwrap();
        assert_eq!(read_u32(&message, 0).unwrap() as usize, message.len());
        assert_eq!(read_u16(&message, 4).unwrap(), GENL_ID_CTRL);
        assert_eq!(read_u16(&message, 6).unwrap(), NLM_F_REQUEST | NLM_F_ACK);
        assert_eq!(message[NLMSG_HEADER_LEN], CTRL_CMD_GETFAMILY);
        let (kind, payload) = first_attr(attributes(&message));
        assert_eq!(kind, CTRL_ATTR_FAMILY_NAME);
        assert_eq!(payload, b"ethtool\0");
        assert_eq!(message.len() % 4, 0);
    }

    #[test]
    fn a_features_request_names_the_device_and_lists_the_features_with_no_value() {
        let message = features_off_request(27, 42, &["tx-checksum-ip-generic"]).unwrap();
        assert_eq!(read_u32(&message, 0).unwrap() as usize, message.len());
        assert_eq!(read_u16(&message, 4).unwrap(), 27);
        assert_eq!(message[NLMSG_HEADER_LEN], ETHTOOL_MSG_FEATURES_SET);
        assert_eq!(message[NLMSG_HEADER_LEN + 1], ETHTOOL_GENL_VERSION);

        let attrs = attributes(&message);
        let (kind, header) = first_attr(attrs);
        assert_eq!(kind, ETHTOOL_A_FEATURES_HEADER | NLA_F_NESTED);
        let (kind, dev_index) = first_attr(header);
        assert_eq!(kind, ETHTOOL_A_HEADER_DEV_INDEX);
        assert_eq!(dev_index, 42u32.to_ne_bytes());
        let (kind, flags) = first_attr(&header[8..]);
        assert_eq!(kind, ETHTOOL_A_HEADER_FLAGS);
        assert_eq!(flags, ETHTOOL_FLAG_OMIT_REPLY.to_ne_bytes());

        let header_len = usize::from(read_u16(attrs, 0).unwrap());
        let (kind, wanted) = first_attr(&attrs[header_len..]);
        assert_eq!(kind, ETHTOOL_A_FEATURES_WANTED | NLA_F_NESTED);
        let (kind, bits) = first_attr(wanted);
        assert_eq!(kind, ETHTOOL_A_BITSET_BITS | NLA_F_NESTED);
        let (kind, bit) = first_attr(bits);
        assert_eq!(kind, ETHTOOL_A_BITSET_BITS_BIT | NLA_F_NESTED);
        let (kind, name) = first_attr(bit);
        assert_eq!(kind, ETHTOOL_A_BITSET_BIT_NAME);
        assert_eq!(name, b"tx-checksum-ip-generic\0");
        // No ETHTOOL_A_BITSET_BIT_VALUE: the feature is turned off.
        assert_eq!(
            bit.len(),
            NLA_HEADER_LEN + "tx-checksum-ip-generic\0".len() + 1
        );
    }

    #[test]
    fn every_tx_checksum_feature_is_turned_off() {
        let message = features_off_request(27, 1, TX_CHECKSUM_FEATURES).unwrap();
        for feature in TX_CHECKSUM_FEATURES {
            let name = c_string(feature);
            assert!(
                message.windows(name.len()).any(|window| window == name),
                "{feature} missing"
            );
        }
    }

    /// A reply datagram of one message of type `kind` with `payload`.
    fn reply_datagram(kind: u16, seq: u32, payload: &[u8]) -> Vec<u8> {
        let mut datagram = Vec::new();
        let len = u32::try_from(NLMSG_HEADER_LEN + payload.len()).unwrap();
        datagram.extend_from_slice(&len.to_ne_bytes());
        datagram.extend_from_slice(&kind.to_ne_bytes());
        datagram.extend_from_slice(&0u16.to_ne_bytes());
        datagram.extend_from_slice(&seq.to_ne_bytes());
        datagram.extend_from_slice(&0u32.to_ne_bytes());
        datagram.extend_from_slice(payload);
        datagram
    }

    #[test]
    fn the_family_id_is_read_from_the_controller_reply() {
        let mut payload = vec![1, 2, 0, 0];
        payload.extend_from_slice(&12u16.to_ne_bytes()); // name attribute: len 12
        payload.extend_from_slice(&CTRL_ATTR_FAMILY_NAME.to_ne_bytes());
        payload.extend_from_slice(b"ethtool\0");
        payload.extend_from_slice(&6u16.to_ne_bytes());
        payload.extend_from_slice(&CTRL_ATTR_FAMILY_ID.to_ne_bytes());
        payload.extend_from_slice(&21u16.to_ne_bytes());
        payload.extend_from_slice(&[0, 0]);
        let datagram = reply_datagram(GENL_ID_CTRL, FAMILY_SEQ, &payload);
        let messages = replies(&datagram).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(family_id(&messages[0]), Some(21));
    }

    #[test]
    fn an_ack_for_the_request_is_success_and_an_errno_is_an_error() {
        let ack = reply_datagram(NLMSG_ERROR, FEATURES_SEQ, &0i32.to_ne_bytes());
        let messages = replies(&ack).unwrap();
        assert!(matches!(
            acknowledgement(&messages[0], FEATURES_SEQ),
            Some(Ok(()))
        ));

        let error = reply_datagram(NLMSG_ERROR, FEATURES_SEQ, &(-19i32).to_ne_bytes());
        let messages = replies(&error).unwrap();
        let result = acknowledgement(&messages[0], FEATURES_SEQ).unwrap();
        assert!(result.unwrap_err().to_string().contains("No such device"));
    }

    #[test]
    fn an_ack_for_another_request_is_not_the_answer() {
        let ack = reply_datagram(NLMSG_ERROR, FAMILY_SEQ, &0i32.to_ne_bytes());
        let messages = replies(&ack).unwrap();
        assert!(acknowledgement(&messages[0], FEATURES_SEQ).is_none());
    }

    #[test]
    fn a_truncated_reply_is_refused() {
        let mut datagram = reply_datagram(NLMSG_ERROR, 1, &0i32.to_ne_bytes());
        datagram[0] = 200;
        assert!(replies(&datagram).is_err());
    }

    /// The kernel's reply to a features request, listing `bits` as the active features, each
    /// by name with whether it carries a value flag. Without `mask` the bitset is a list of the
    /// active bits, as the kernel sends the active features.
    fn features_reply(bits: &[(&str, bool)], mask: bool) -> Vec<u8> {
        let mut message = MessageBuilder::new(
            27,
            FEATURES_SEQ,
            ETHTOOL_MSG_FEATURES_GET_REPLY,
            ETHTOOL_GENL_VERSION,
        );
        let header = message.start_nested(ETHTOOL_A_FEATURES_HEADER);
        message
            .attr(ETHTOOL_A_HEADER_DEV_INDEX, &42u32.to_ne_bytes())
            .unwrap();
        message.end_nested(header).unwrap();
        let active = message.start_nested(ETHTOOL_A_FEATURES_ACTIVE);
        if !mask {
            message.attr(ETHTOOL_A_BITSET_NOMASK, &[]).unwrap();
        }
        message
            .attr(ETHTOOL_A_BITSET_SIZE, &64u32.to_ne_bytes())
            .unwrap();
        let list = message.start_nested(ETHTOOL_A_BITSET_BITS);
        for (index, (name, value)) in bits.iter().enumerate() {
            let bit = message.start_nested(ETHTOOL_A_BITSET_BITS_BIT);
            message
                .attr(
                    ETHTOOL_A_BITSET_BIT_INDEX,
                    &u32::try_from(index).unwrap().to_ne_bytes(),
                )
                .unwrap();
            message
                .attr(ETHTOOL_A_BITSET_BIT_NAME, &c_string(name))
                .unwrap();
            if *value {
                message.attr(ETHTOOL_A_BITSET_BIT_VALUE, &[]).unwrap();
            }
            message.end_nested(bit).unwrap();
        }
        message.end_nested(list).unwrap();
        message.end_nested(active).unwrap();
        message.finish().unwrap()
    }

    fn offload_in(datagram: &[u8]) -> Result<bool> {
        let messages = replies(datagram).unwrap();
        features_state(&messages[0]).unwrap()
    }

    #[test]
    fn a_features_get_request_names_the_device_and_asks_for_a_reply() {
        let message = features_get_request(27, 42).unwrap();
        assert_eq!(read_u32(&message, 0).unwrap() as usize, message.len());
        assert_eq!(read_u16(&message, 4).unwrap(), 27);
        assert_eq!(message[NLMSG_HEADER_LEN], ETHTOOL_MSG_FEATURES_GET);
        let attrs = attributes(&message);
        let (kind, header) = first_attr(attrs);
        assert_eq!(kind, ETHTOOL_A_FEATURES_HEADER | NLA_F_NESTED);
        let (kind, dev_index) = first_attr(header);
        assert_eq!(kind, ETHTOOL_A_HEADER_DEV_INDEX);
        assert_eq!(dev_index, 42u32.to_ne_bytes());
        // Only the device: no flags, so the reply is sent, with bitsets listed by name.
        assert_eq!(header.len(), NLA_HEADER_LEN + 4);
        assert_eq!(attrs.len(), NLA_HEADER_LEN + header.len());
    }

    #[test]
    fn an_active_tx_checksum_feature_is_offload_on() {
        let reply = features_reply(
            &[
                ("tx-scatter-gather", false),
                ("tx-checksum-ip-generic", false),
            ],
            false,
        );
        assert!(offload_in(&reply).unwrap());
    }

    #[test]
    fn active_features_with_no_tx_checksum_feature_are_offload_off() {
        let reply = features_reply(
            &[("tx-scatter-gather", false), ("rx-checksum", false)],
            false,
        );
        assert!(!offload_in(&reply).unwrap());
        assert!(!offload_in(&features_reply(&[], false)).unwrap());
    }

    #[test]
    fn in_a_masked_bitset_only_a_bit_with_a_value_is_on() {
        let off = features_reply(&[("tx-checksum-ipv4", false)], true);
        assert!(!offload_in(&off).unwrap());
        let on = features_reply(&[("tx-checksum-ipv4", true)], true);
        assert!(offload_in(&on).unwrap());
    }

    #[test]
    fn an_error_for_the_features_request_is_an_error() {
        let error = reply_datagram(NLMSG_ERROR, FEATURES_SEQ, &(-19i32).to_ne_bytes());
        let error = offload_in(&error).unwrap_err();
        assert!(error.to_string().contains("No such device"), "{error:#}");
        let ack = reply_datagram(NLMSG_ERROR, FEATURES_SEQ, &0i32.to_ne_bytes());
        assert!(offload_in(&ack).is_err());
    }

    #[test]
    fn a_reply_with_no_active_features_is_an_error() {
        let mut payload = vec![ETHTOOL_MSG_FEATURES_GET_REPLY, ETHTOOL_GENL_VERSION, 0, 0];
        payload.extend_from_slice(&8u16.to_ne_bytes());
        payload.extend_from_slice(&ETHTOOL_A_HEADER_DEV_INDEX.to_ne_bytes());
        payload.extend_from_slice(&42u32.to_ne_bytes());
        let reply = reply_datagram(27, FEATURES_SEQ, &payload);
        assert!(offload_in(&reply).is_err());
    }

    #[test]
    fn a_reply_to_another_request_is_not_the_answer() {
        let reply = features_reply(&[("tx-checksum-ip-generic", false)], false);
        let messages = replies(&reply).unwrap();
        let other = Reply {
            kind: messages[0].kind,
            seq: FAMILY_SEQ,
            payload: messages[0].payload,
        };
        assert!(features_state(&other).is_none());
    }

    #[tokio::test]
    #[ignore = "requires root: makes a veth and changes its offload"]
    async fn a_new_veth_has_tx_checksum_offload_on_until_it_is_turned_off() {
        let (a, b) = ("it-txcsum-a", "it-txcsum-b");
        let _ = delete_interface(a).await;
        create_veth_pair(a, b, a, b).await.unwrap();
        let ifindex = get_ifindex(a).await.unwrap();

        assert!(tx_checksum_offload_on(ifindex).unwrap(), "a new veth");
        disable_tx_checksum_offload(ifindex).unwrap();
        assert!(
            !tx_checksum_offload_on(ifindex).unwrap(),
            "after turning it off"
        );
        disable_tx_checksum_offload(ifindex).unwrap();
        assert!(
            !tx_checksum_offload_on(ifindex).unwrap(),
            "turned off twice"
        );

        delete_interface(a).await.unwrap();
    }
}
