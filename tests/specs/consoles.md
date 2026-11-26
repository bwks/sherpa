# Consoles

Opening a node's console: the serial console of a virtual machine or unikernel in the user's terminal with `sherpa console`, a virtual machine's VNC console in the user's own VNC viewer with `sherpa vnc`, both relayed by sherpad over its API; who may open one, how many sessions a console serves, what is refused, how an open console ends, the sockets the consoles are on the server, and nodes defined by an earlier release. A container has neither console, and the web UI opens none. An admin being refused a console is `access_control_007` in [accounts-and-auth.md](accounts-and-auth.md). Lab lifecycle, nodes, accounts and what inspect shows have their own specs.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `serial_console_002_ctrl_right_bracket_ends_the_console_with_exit_code_0`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real libvirt, Docker and the database, unit only for logic with too many branches to cover above.

## `serial_console`: `sherpa console` puts a node's serial console in the user's terminal

**Statement:** `sherpa console <node>` puts the serial console of a running virtual machine or unikernel in its owner's terminal, through the server, until they leave it.
**Relied on by:** the user, most of all when a node has no network or no SSH yet. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | With no `telnet` or `ssh` on `PATH`, the owner opens a VM's console: the connect line names the node, the lab and both escapes, the node's login prompt shows and typed text is echoed | e2e |
| 002 | `Ctrl+]` ends the console with exit code 0 and one close line, on a line of its own | e2e |
| 003 | `~.` at the start of a line ends it the same way | e2e |
| 004 | The owner opens a unikernel's serial console and reads what it prints (no test yet) | e2e |
| 005 | The console opens in a folder made by `sherpa download`, which has no manifest (no test yet) | e2e |
| 006 | Each console is asked for at the server the other commands use, with the same scheme, host and port, and with the lab and the node each one path segment whatever characters the name has | unit |
| 007 | A console that cannot be reached, by a timeout or a TLS failure, fails with an error that carries the console and the node, with the cause kept | unit |
| 008 | Leaving a console does not log the node's session out: the next console finds it as it was left (no test yet) | e2e |
| 009 | On Windows, in Windows Terminal and in conhost, the console opens, the node's prompt shows and `Ctrl+]` ends it (no test yet) | e2e |

## `vnc_console`: `sherpa vnc` relays a VM's VNC console to a viewer on the user's machine

**Statement:** `sherpa vnc <node>` relays the VNC console of a running virtual machine to VNC viewers on the user's own machine, through the server, until Ctrl+C.
**Relied on by:** the user, for nodes with a graphical console and nodes with no SSH yet. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpa vnc` prints an address on `127.0.0.1`, and a viewer that connects to it gets the node's RFB banner | e2e |
| 002 | Ctrl+C stops the command with exit code 0 and its stop line | e2e |
| 003 | Whatever the server refuses the console for as the command starts, not the owner, no VNC console, a node not running or not logged in, the command ends with the reason and exit code 1 | unit |
| 004 | A server that cannot be reached as the command starts ends it, with the cause | unit |
| 005 | Started while another session holds the console, it prints its address, says who holds it and how to take it over, and keeps listening | e2e |
| 006 | A viewer that connects while the console is held gets nothing; the command says who holds it and goes on listening | e2e |
| 007 | Once a viewer leaves, the next one gets the banner: a viewer can disconnect and connect again | e2e |
| 008 | With `--force`, a viewer that connects takes the console over from the owner's other viewer, which is closed and told who took it (no test at this level yet) | e2e |
| 009 | A viewer refused for a reason that asking again cannot change, the login, the owner or the node, ends the command with that reason | unit |
| 010 | A Ctrl+C that comes just as a viewer's connection ends still stops the command | unit |
| 011 | What the viewer sends reaches the node: its reply to the banner is answered (no test at this level yet) | e2e |
| 012 | When the server closes a viewer's session, the viewer's connection is closed with nothing more sent | unit |
| 013 | The end of each viewer's connection is told with the viewer's address: the viewer left, or the server closed it and why | unit |
| 014 | On Windows, `sherpa vnc` prints its address, a viewer gets the banner and Ctrl+C stops it (no test yet) | e2e |

## `console_access`: only a lab's owner opens its consoles

**Statement:** a node's serial and VNC consoles are opened only by the owner of its lab, with the token of their login.
**Relied on by:** the user, as a console reaches a node without its login. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | Another user, in a folder holding a copy of the owner's lab info, is refused `sherpa console` by the server, as a lab that does not exist | e2e |
| 002 | Another user and an admin are each refused `sherpa vnc`, the user as a lab that does not exist and the admin as not the lab's owner, with exit code 1 and no address printed | e2e |
| 003 | A request with no token is refused with 401, for each console | integration |
| 004 | The web UI's session cookie opens no console: a request with the owner's cookie and no bearer token is refused with 401 (no test yet) | integration |
| 005 | A token whose session has ended opens no console (no test yet) | integration |
| 006 | Each console opened, refused and closed is logged with the user, the lab, the node, the console and the client's address, and never the bytes (no test yet) | integration |

## `console_session`: a console serves one session at a time

**Statement:** a console serves one session at a time, and its owner can take it over from a session of their own.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A second `sherpa console` on a console in use fails at once, naming who holds it and offering `--force` | e2e |
| 002 | `--force` takes the console over, at the node's prompt, and the session taken over ends with exit code 0, saying who took it | e2e |
| 003 | A node's serial and VNC consoles are held apart: both are open at once (no test at this level yet) | e2e |
| 004 | The consoles of two nodes, and of nodes of one name in two labs, are held apart | unit |
| 005 | A session that was taken over, and ends after the new one holds the console, does not let go of the new one's | unit |
| 006 | `--force` takes over only a session of the same user | unit |
| 007 | A holder is named by its user, the client's address when the server knows it, and when the session began, in UTC | unit |
| 008 | A check of a console answers who holds it, and neither takes it nor ends the holder's session (no test at this level yet) | integration |

## `console_refusal`: a console that cannot be opened is refused with the reason

**Statement:** a console that cannot be opened is refused before anything connects, with the reason and what to do instead.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpa console` on a container fails, saying containers have no serial console and to use `sherpa ssh` | e2e |
| 002 | `sherpa vnc` on a container ends with exit code 1, saying containers have no VNC console, and prints no address | e2e |
| 003 | A unikernel's VNC console is refused, pointing to `sherpa console` (no test at this level yet) | integration |
| 004 | After down, both consoles of a node are refused as not running, saying to resume (no test at this level yet) | e2e |
| 005 | A lab or a node that does not exist is answered not found, for each console | integration |
| 006 | A request that is neither a WebSocket upgrade nor a check is refused as a bad request | integration |
| 007 | A running node with no serial device, or no VNC graphics, that Sherpa can reach is refused as having none | unit |
| 008 | The CLI tells a refusal in the server's own words, from its details when it gives them; a 401 as not logged in, saying to run `sherpa login`; and an answer with no words of the server's by its HTTP status | unit |
| 009 | A server with no console route, one older than the client, is told as that for each console, saying to upgrade the server | unit |
| 010 | Opening a console is refused as its check is, with the same status and the same words (no test yet) | integration |
| 011 | A node VM's VNC console is refused, pointing to `sherpa console`, and its serial console is not | unit |

## `console_end`: an open console ends when its node stops or its login ends, and says why

**Statement:** an open console ends when its node stops, when its user's login ends and when sherpad stops, and its user is told why.
**Relied on by:** the user, the admin who ends a user's sessions. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | Down ends an open serial console with exit code 0, saying the node stopped, and after resume the console opens again (no test yet) | e2e |
| 002 | Redeploying a node ends its open console the same way, and a console opened after is the new node's (no test yet) | e2e |
| 003 | Destroy ends the open consoles of the lab (no test yet) | e2e |
| 004 | A viewer connected when its node stops is closed and `sherpa vnc` says so and keeps listening; after resume a viewer connects again (no test yet) | e2e |
| 005 | A console open when its user logs out ends within 30 seconds, saying the login ended and to log in again (no test yet) | e2e |
| 006 | A restart of sherpad ends the open consoles and leaves the nodes running; after it each console opens again, held by no one (no test yet) | e2e |
| 007 | Each way the server ends a session has its own close code and reason: 1000 when the node's console closed, 4001 naming who took it over, 4002 when the login ended, 1011 when the relay failed; and a VNC console the node closed is not said to have stopped | unit |
| 008 | A reason longer than a close frame carries is cut short, at a character boundary | unit |

## `console_sockets`: a node's consoles are sockets only sherpad reaches

**Statement:** each console is a UNIX socket on the server that only sherpad and the `kvm` group reach, and no console listens on a TCP port.
**Relied on by:** the user, against the other accounts on the server. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | The `sherpa` account opens a running node's serial and VNC sockets, and an OS account in neither `sherpa` nor `kvm` is refused both | integration |
| 002 | A running node's consoles listen on no TCP port: nothing in `127.127.0.0/16`, and nothing on ports 5900 to 6999 | integration |
| 003 | A VM is defined with its serial console and its VNC console on the node's own sockets | unit |
| 004 | A unikernel is defined with its serial console on its socket, and no VNC console | unit |
| 005 | A lab's console folder is made for `sherpa` and `kvm` only, mode 2770 | unit |
| 006 | Destroy removes the lab's console folder with any socket left in it, and a folder already gone is not an error (no test at this level yet) | integration |
| 007 | A destroy that cannot remove the console folder reports it, exits non-zero and keeps the lab's record (no test yet) | integration |
| 008 | A node Sherpa itself brought up has its console sockets in its lab's folder, which no account outside `sherpa` and `kvm` can enter (no test yet) | integration |

## `console_upgrade`: a node defined with its consoles on TCP moves to its sockets at its next start

**Statement:** a node defined by an earlier release with its consoles on TCP listeners comes up with them on its sockets at its next start, and is refused its consoles until then.
**Relied on by:** the user after an update, the admin. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | A stored definition with a telnet serial console is rewritten with it on the node's socket, and everything else as it was | unit |
| 002 | A stored definition with VNC on a TCP port is rewritten with it on the node's socket, and everything else as it was | unit |
| 003 | Both consoles move in one definition, and one moves alone when only it is still on TCP | unit |
| 004 | A definition with its consoles on sockets, or with no serial device or no VNC graphics, is left as it is | unit |
| 005 | The elements are rewritten in each form libvirt and Sherpa wrote them: a source with children, a protocol with an end tag, an empty graphics element, and listen elements beside other children | unit |
| 006 | A definition that is not well-formed XML is refused, not rewritten | unit |
| 007 | How a running node's console is reached is read from its first serial device and its first VNC graphics: its socket, a TCP listener, or another device | unit |
| 008 | A running node whose console is still on TCP is refused it, the serial and the VNC console each, saying to down and resume the lab or redeploy the node | unit |
| 009 | A domain defined with a telnet console and a VNC port, moved and then started, runs with both on its sockets, and moving it again changes nothing | integration |
| 010 | Resume moves a shut-off node's consoles before it boots, and makes the lab's console folder when it is missing (no test yet) | integration |
| 011 | When sherpad starts, it moves the consoles of every shut-off node and makes each lab's console folder, and leaves a running node as it is (no test yet) | integration |
